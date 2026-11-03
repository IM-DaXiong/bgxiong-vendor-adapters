"""Mask artifact export/load and bounded progress facts (video-target-keyframes-v1).

Package-owned on-disk format under Comfy output/input:
  bgx-wan22-tracking/<requestId>/
    RUNNING                 # present while request is in flight
    progress.json           # SPEC TrackingProgress facts (real counts only)
    manifest.json           # sealed after export; digest covers mask payload
    masks/frame_NNNNN.png   # uint8 0/255 single-channel
    preview/overlay.png     # contact-sheet preview (opaque package asset)
    preview/index.json      # per-frame preview index

Atomic writes use temp + os.replace. Finished artifacts are never deleted here:
a confirmed mask may still be frozen into a generation, and this package has no
reliable release signal. Nothing here touches the application database.
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import re
import shutil
import tempfile
import time
from pathlib import Path, PurePosixPath
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

import numpy as np

try:
    from PIL import Image
except ImportError:  # pragma: no cover
    Image = None  # type: ignore

POLICY = "mask-artifact-v1"
BASE_HTTP = "/bgx/wan22-animate/tracking"
DIR_NAME = "bgx-wan22-tracking"
MANIFEST = "manifest.json"
PROGRESS = "progress.json"
RUNNING = "RUNNING"
MASKS = "masks"
PREVIEW = "preview"
_REQUEST_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")


def validate_request_id(request_id: str) -> str:
    if not isinstance(request_id, str) or not _REQUEST_ID_RE.match(request_id):
        raise ValueError("requestId must be a short alphanumeric token")
    if ".." in request_id or "/" in request_id or "\\" in request_id:
        raise ValueError("requestId must not contain path separators")
    return request_id


def artifact_root(base: Path, request_id: str) -> Path:
    rid = validate_request_id(request_id)
    root = (Path(base) / DIR_NAME / rid).resolve()
    base_res = Path(base).resolve()
    if not root.is_relative_to(base_res / DIR_NAME):
        raise ValueError("artifact path escaped package directory")
    return root


def _atomic_write_bytes(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".tmp-", suffix=".part", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as out:
            out.write(data)
            out.flush()
            os.fsync(out.fileno())
        os.replace(tmp, path)
        tmp = None
    finally:
        if tmp is not None:
            Path(tmp).unlink(missing_ok=True)


def _atomic_write_json(path: Path, payload: dict) -> None:
    raw = (json.dumps(payload, ensure_ascii=False, separators=(",", ":"), sort_keys=True) + "\n").encode("utf-8")
    _atomic_write_bytes(path, raw)


def mark_running(root: Path) -> None:
    root.mkdir(parents=True, exist_ok=True)
    (root / RUNNING).write_text(str(int(time.time())), encoding="utf-8")


def clear_running(root: Path) -> None:
    (root / RUNNING).unlink(missing_ok=True)


def is_running(root: Path) -> bool:
    return (root / RUNNING).is_file()


def write_progress(
    root: Path,
    *,
    request_id: str,
    revision: str,
    selection_digest: str,
    stage: str,
    direction: str,
    total_frames: Optional[int],
    processed_frames: Optional[int],
    last_completed_time_ms: Optional[int],
    covered_ranges: Sequence[dict],
) -> dict:
    """Write SPEC progress facts. Only real counts; never invent percentages."""
    if stage not in ("queued", "loading", "towardEnd", "towardStart", "finalizing"):
        raise ValueError(f"unknown progress stage {stage}")
    if direction not in ("towardStart", "towardEnd", "both"):
        raise ValueError(f"unknown direction {direction}")
    payload = {
        "requestId": validate_request_id(request_id),
        "revision": str(revision),
        "selectionDigest": str(selection_digest),
        "stage": stage,
        "direction": direction,
        "totalFrames": total_frames,
        "processedFrames": processed_frames,
        "lastCompletedTimeMs": last_completed_time_ms,
        "coveredRanges": list(covered_ranges),
    }
    root.mkdir(parents=True, exist_ok=True)
    _atomic_write_json(root / PROGRESS, payload)
    return payload


class ProgressReporter:
    """Per-frame propagation facts. Counts each covered frame once (anchors included once even
    when both passes reach them), writes atomically at most every `min_interval` seconds or on a
    stage change, and always on `flush`. Never reports a frame that was not propagated."""

    def __init__(self, root: Path, *, request_id: str, selection_digest: str, direction: str,
                 frame_times: Sequence[int], coverage: Sequence[int], span_ms: int,
                 clock=time.monotonic, min_interval: float = 0.5):
        self.root = root
        self.request_id = validate_request_id(request_id)
        self.selection_digest = selection_digest
        self.direction = direction
        self.frame_times = [int(t) for t in frame_times]
        self.coverage = set(int(i) for i in coverage)
        self.span_ms = int(span_ms)
        self.clock = clock
        self.min_interval = min_interval
        self.done: set = set()
        self.stage_name = "loading"
        self.last_frame: Optional[int] = None
        self.last_write: Optional[float] = None
        previous = read_progress(root) or {}
        try:
            self.revision = int(previous.get("revision", "0"))
        except (TypeError, ValueError):
            self.revision = 0

    def _ranges(self) -> List[dict]:
        if not self.done:
            return []
        ordered = sorted(self.done)
        out = []
        start = prev = ordered[0]
        for i in ordered[1:]:
            if i == prev + 1:
                prev = i
                continue
            out.append(self._range(start, prev))
            start = prev = i
        out.append(self._range(start, prev))
        return out

    def _range(self, first: int, last: int) -> dict:
        end = self.frame_times[last + 1] if last + 1 < len(self.frame_times) else self.span_ms
        return {"startMs": self.frame_times[first], "endMs": end}

    def _write(self) -> dict:
        self.revision += 1
        self.last_write = self.clock()
        return write_progress(
            self.root,
            request_id=self.request_id,
            revision=str(self.revision),
            selection_digest=self.selection_digest,
            stage=self.stage_name,
            direction=self.direction,
            total_frames=len(self.coverage),
            processed_frames=len(self.done),
            last_completed_time_ms=None if self.last_frame is None else self.frame_times[self.last_frame],
            covered_ranges=self._ranges(),
        )

    def stage(self, stage: str) -> dict:
        self.stage_name = stage
        return self._write()

    def frame(self, stage: str, frame_index: int) -> Optional[dict]:
        idx = int(frame_index)
        if idx < 0 or idx >= len(self.frame_times):
            raise ValueError(f"propagated frame {idx} has no frame time")
        if idx in self.coverage:
            self.done.add(idx)
        self.last_frame = idx
        if stage != self.stage_name:
            self.stage_name = stage
            return self._write()
        if self.last_write is None or self.clock() - self.last_write >= self.min_interval:
            return self._write()
        return None

    def flush(self) -> dict:
        return self._write()


def read_progress(root: Path) -> Optional[dict]:
    path = root / PROGRESS
    if not path.is_file():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def _mask_to_u8(mask: np.ndarray) -> np.ndarray:
    arr = np.asarray(mask)
    if arr.ndim != 2:
        raise ValueError(f"mask must be 2-D, got {arr.shape}")
    if arr.dtype == np.bool_ or arr.dtype == np.uint8:
        out = (arr > 0).astype(np.uint8) * 255
    else:
        out = (arr > 0.5).astype(np.uint8) * 255
    return out


def _sha256_bytes(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _frame_name(index: int) -> str:
    return f"frame_{int(index):05d}.png"


def export_masks(
    base: Path,
    *,
    request_id: str,
    selection_digest: str,
    direction: str,
    width: int,
    height: int,
    fps: float,
    masks: Sequence[np.ndarray],
    covered_indices: Sequence[int],
    resolved_keyframes: Sequence[dict],
    source_digest: str,
    range_ms: dict,
    operation_ref: dict,
    frame_times: Sequence[int],
    revision: str = "1",
) -> dict:
    """Atomically export mask frames + manifest + preview. Returns maskArtifact + receipt fields.

    `covered_indices` are the propagated frames; a propagated frame whose mask is empty is `lost`.
    `frame_times` are the actual presentation times (ms from range start) of every frame.
    """
    if Image is None:
        raise RuntimeError("Pillow is required for mask artifact export")
    rid = validate_request_id(request_id)
    if direction not in ("towardStart", "towardEnd", "both"):
        raise ValueError(f"unknown direction {direction}")
    if len(masks) == 0:
        raise ValueError("masks must be non-empty")
    frame_count = len(masks)
    times = [int(t) for t in frame_times]
    span_ms = int(range_ms["endMs"]) - int(range_ms["startMs"])
    if len(times) != frame_count or any(b <= a for a, b in zip(times, times[1:])):
        raise ValueError("frame_times must give one increasing time per mask frame")
    if times[0] < 0 or times[-1] >= span_ms:
        raise ValueError("frame_times must lie inside the range")
    computed = sorted({int(i) for i in covered_indices})
    for i in computed:
        if i < 0 or i >= frame_count:
            raise ValueError(f"covered index {i} outside 0..{frame_count - 1}")
    lost = [i for i in computed if not np.any(np.asarray(masks[i]) > 0.5)]
    covered = [i for i in computed if i not in set(lost)]

    root = artifact_root(base, rid)
    mark_running(root)
    try:
        previous = read_progress(root) or {}
        revision = str(int(previous.get("revision", revision)) + 1)
        write_progress(
            root,
            request_id=rid,
            revision=revision,
            selection_digest=selection_digest,
            stage="finalizing",
            direction=direction,
            total_frames=len(computed),
            processed_frames=len(computed),
            last_completed_time_ms=None if not computed else times[computed[-1]],
            covered_ranges=_indices_to_ranges(computed, times, span_ms),
        )

        masks_dir = root / MASKS
        if masks_dir.exists():
            shutil.rmtree(masks_dir)
        masks_dir.mkdir(parents=True, exist_ok=True)

        hasher = hashlib.sha256()
        frame_entries: List[dict] = []
        u8_frames: List[np.ndarray] = []
        for idx, mask in enumerate(masks):
            u8 = _mask_to_u8(mask)
            if u8.shape != (height, width) and (height > 0 and width > 0):
                # Prefer declared geometry; fall back to actual if caller passed 0.
                if height > 0 and width > 0:
                    if u8.shape[0] != height or u8.shape[1] != width:
                        raise ValueError(
                            f"mask {idx} shape {u8.shape} != declared ({height}, {width})"
                        )
            u8_frames.append(u8)
            raw = u8.tobytes()
            hasher.update(f"{idx}:".encode("ascii"))
            hasher.update(raw)
            name = _frame_name(idx)
            img = Image.fromarray(u8, mode="L")
            buf = tempfile.NamedTemporaryFile(delete=False, dir=masks_dir, suffix=".png")
            try:
                img.save(buf, format="PNG")
                buf.flush()
                os.fsync(buf.fileno())
                buf.close()
                os.replace(buf.name, masks_dir / name)
            finally:
                try:
                    os.unlink(buf.name)
                except FileNotFoundError:
                    pass
            state = "covered" if idx in covered else "lost" if idx in lost else "uncomputed"
            frame_entries.append(
                {
                    "frameIndex": idx,
                    "timeMs": times[idx],
                    "state": state,
                    "relPath": f"{MASKS}/{name}",
                }
            )

        digest = "sha256:" + hasher.hexdigest()
        preview_rel, preview_index = _write_preview(root, u8_frames, covered)

        h = int(u8_frames[0].shape[0])
        w = int(u8_frames[0].shape[1])
        uncomputed = [i for i in range(frame_count) if i not in set(computed)]
        ready = len(covered) > 0 and not uncomputed
        if not covered:
            ready_reason = "no covered frames"
        elif uncomputed:
            ready_reason = "uncomputed frames remain"
        else:
            ready_reason = None

        manifest = {
            "policy": POLICY,
            "requestId": rid,
            "selectionDigest": selection_digest,
            "sourceDigest": source_digest,
            "range": range_ms,
            "direction": direction,
            "revision": revision,
            "operationRef": operation_ref,
            "outputGeometry": {"width": w, "height": h},
            "outputFps": float(fps) if fps > 0 else None,
            "resolvedKeyframes": list(resolved_keyframes),
            "coveredRanges": _indices_to_ranges(covered, times, span_ms),
            "lostRanges": _indices_to_ranges(lost, times, span_ms),
            "uncomputedRanges": _indices_to_ranges(uncomputed, times, span_ms),
            "skipRanges": [],
            "ready": ready,
            "readyReason": ready_reason,
            "previewMediaRef": preview_rel,
            "frameCount": frame_count,
            "frames": frame_entries,
            "previewIndex": preview_index,
            "digest": digest,
        }
        _atomic_write_json(root / MANIFEST, manifest)
        clear_running(root)
        return {
            "artifactId": rid,
            "digest": digest,
            "pageIndexRef": f"{DIR_NAME}/{rid}/{MANIFEST}",
            "manifest": manifest,
            "previewMediaRef": preview_rel,
        }
    except Exception:
        # Leave RUNNING so bounded GC does not delete a half-written tree mid-failure
        # unless caller explicitly clears; mark failed progress if possible.
        try:
            write_progress(
                root,
                request_id=rid,
                revision=revision,
                selection_digest=selection_digest,
                stage="finalizing",
                direction=direction,
                total_frames=len(masks),
                processed_frames=None,
                last_completed_time_ms=None,
                covered_ranges=[],
            )
        except Exception:
            pass
        raise


def load_masks(base: Path, artifact_id: str, digest: str) -> Tuple[List[np.ndarray], dict]:
    """Load masks and verify digest. Raises on mismatch."""
    if Image is None:
        raise RuntimeError("Pillow is required for mask artifact load")
    root = artifact_root(base, artifact_id)
    if is_running(root):
        raise ValueError("mask artifact is still being written")
    man_path = root / MANIFEST
    if not man_path.is_file():
        raise FileNotFoundError("mask artifact manifest missing")
    manifest = json.loads(man_path.read_text(encoding="utf-8"))
    if manifest.get("digest") != digest:
        raise ValueError("mask artifact digest mismatch")
    hasher = hashlib.sha256()
    out: List[np.ndarray] = []
    for entry in manifest["frames"]:
        rel = entry["relPath"]
        path = (root / PurePosixPath(rel)).resolve()
        if not path.is_relative_to(root) or not path.is_file():
            raise ValueError(f"mask frame missing or escaped: {rel}")
        with Image.open(path) as im:
            u8 = np.array(im.convert("L"), dtype=np.uint8)
        idx = int(entry["frameIndex"])
        hasher.update(f"{idx}:".encode("ascii"))
        hasher.update(u8.tobytes())
        out.append((u8 > 0).astype(np.float32))
    actual = "sha256:" + hasher.hexdigest()
    if actual != digest:
        raise ValueError("mask artifact payload digest mismatch")
    return out, manifest


def _indices_to_ranges(indices: Sequence[int], times: Sequence[int], span_ms: int) -> List[dict]:
    "Contiguous frame runs as [first frame time, next frame time or range end)."
    if not indices:
        return []
    ordered = sorted(int(i) for i in indices)
    ranges = []
    start = prev = ordered[0]
    for i in ordered[1:]:
        if i == prev + 1:
            prev = i
            continue
        ranges.append(_range_ms(start, prev, times, span_ms))
        start = prev = i
    ranges.append(_range_ms(start, prev, times, span_ms))
    return ranges


def _range_ms(start_idx: int, end_idx: int, times: Sequence[int], span_ms: int) -> dict:
    end = times[end_idx + 1] if end_idx + 1 < len(times) else span_ms
    return {"startMs": int(times[start_idx]), "endMs": int(end)}


def _write_preview(root: Path, frames: Sequence[np.ndarray], covered: Sequence[int]) -> Tuple[str, dict]:
    prev = root / PREVIEW
    if prev.exists():
        shutil.rmtree(prev)
    prev.mkdir(parents=True, exist_ok=True)
    # Contact sheet of up to 16 covered frames (or evenly sampled).
    sample = list(covered) if covered else list(range(min(len(frames), 16)))
    if len(sample) > 16:
        step = max(1, len(sample) // 16)
        sample = sample[::step][:16]
    tiles = []
    for idx in sample:
        u8 = frames[idx]
        rgb = np.stack([u8, np.zeros_like(u8), np.zeros_like(u8)], axis=-1)
        tiles.append((idx, Image.fromarray(rgb, mode="RGB")))
    if not tiles:
        blank = Image.new("RGB", (frames[0].shape[1], frames[0].shape[0]), (0, 0, 0))
        tiles = [(0, blank)]
    tw, th = tiles[0][1].size
    cols = min(4, len(tiles))
    rows = (len(tiles) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * tw, rows * th), (16, 16, 16))
    index = {"tiles": [], "width": cols * tw, "height": rows * th}
    for n, (idx, im) in enumerate(tiles):
        x = (n % cols) * tw
        y = (n // cols) * th
        sheet.paste(im, (x, y))
        index["tiles"].append({"frameIndex": idx, "x": x, "y": y, "w": tw, "h": th})
    out_path = prev / "overlay.png"
    sheet.save(out_path, format="PNG")
    _atomic_write_json(prev / "index.json", index)
    rel = f"{DIR_NAME}/{root.name}/{PREVIEW}/overlay.png"
    return rel, index


# ---- Comfy nodes ----

class BGXAnimateMaskArtifactExport:
    """Export a MASK batch into a package-owned maskArtifact (atomic)."""

    @classmethod
    def INPUT_TYPES(cls):
        return {
            "required": {
                "masks": ("MASK",),
                "request_id": ("STRING", {"default": "tracking-req"}),
                "selection_digest": ("STRING", {"default": ""}),
                "source_digest": ("STRING", {"default": ""}),
                "direction": (["towardStart", "towardEnd", "both"],),
                "covered_indices_json": ("STRING", {"default": "[]"}),
                "resolved_keyframes_json": ("STRING", {"default": "[]"}),
                "range_start_ms": ("INT", {"default": 0, "min": 0, "max": 10**9}),
                "range_end_ms": ("INT", {"default": 0, "min": 0, "max": 10**9}),
                "fps": ("FLOAT", {"default": 0.0, "min": 0.0, "max": 240.0}),
                "frame_times_json": ("STRING", {"default": "[]"}),
                "width": ("INT", {"default": 0, "min": 0, "max": 8192}),
                "height": ("INT", {"default": 0, "min": 0, "max": 8192}),
                "operation_id": ("STRING", {"default": "local.bgx.wan22-animate/tracking-preview"}),
                "operation_version": ("STRING", {"default": "1.0.0"}),
                "input_schema_digest": ("STRING", {"default": ""}),
            }
        }

    RETURN_TYPES = ("STRING", "STRING", "STRING", "STRING")
    RETURN_NAMES = ("artifact_id", "digest", "receipt_json", "progress_json")
    FUNCTION = "export"
    CATEGORY = "bgx/wan22-animate"
    OUTPUT_NODE = True

    def export(
        self,
        masks,
        request_id,
        selection_digest,
        source_digest,
        direction,
        covered_indices_json,
        resolved_keyframes_json,
        range_start_ms,
        range_end_ms,
        fps,
        frame_times_json,
        width,
        height,
        operation_id,
        operation_version,
        input_schema_digest,
    ):
        import folder_paths

        if hasattr(masks, "detach"):
            arr = masks.detach().cpu().float().numpy()
        else:
            arr = np.asarray(masks, dtype=np.float32)
        if arr.ndim == 2:
            frame_list = [arr]
        elif arr.ndim == 3:
            frame_list = [arr[i] for i in range(arr.shape[0])]
        else:
            raise ValueError(f"masks must be [H,W] or [B,H,W], got {arr.shape}")
        covered = json.loads(covered_indices_json.replace("'", '"'))
        if not isinstance(covered, list):
            raise ValueError("covered_indices_json must be a JSON array")
        keyframes = json.loads(resolved_keyframes_json.replace("'", '"'))
        if not isinstance(keyframes, list):
            raise ValueError("resolved_keyframes_json must be a JSON array")
        h = int(height) if int(height) > 0 else int(frame_list[0].shape[0])
        w = int(width) if int(width) > 0 else int(frame_list[0].shape[1])
        # Prefer output directory so artifacts are not mixed with uploads.
        base = Path(folder_paths.get_output_directory())
        result = export_masks(
            base,
            request_id=str(request_id),
            selection_digest=str(selection_digest),
            direction=str(direction),
            width=w,
            height=h,
            fps=float(fps),
            masks=frame_list,
            covered_indices=[int(x) for x in covered],
            resolved_keyframes=keyframes,
            source_digest=str(source_digest),
            range_ms={"startMs": int(range_start_ms), "endMs": int(range_end_ms)},
            operation_ref={
                "operationId": str(operation_id),
                "operationVersion": str(operation_version),
                "inputSchemaDigest": str(input_schema_digest),
            },
            frame_times=json.loads(frame_times_json),
        )
        receipt = {
            "requestId": result["artifactId"],
            "sourceDigest": source_digest,
            "range": {"startMs": int(range_start_ms), "endMs": int(range_end_ms)},
            "direction": direction,
            "selectionDigest": selection_digest,
            "operationRef": {
                "operationId": operation_id,
                "operationVersion": operation_version,
                "inputSchemaDigest": input_schema_digest,
            },
            "outputGeometry": result["manifest"]["outputGeometry"],
            "outputFps": result["manifest"]["outputFps"],
            "resolvedKeyframes": keyframes,
            "coveredRanges": result["manifest"]["coveredRanges"],
            "lostRanges": result["manifest"]["lostRanges"],
            "uncomputedRanges": result["manifest"]["uncomputedRanges"],
            "skipRanges": result["manifest"]["skipRanges"],
            "ready": result["manifest"]["ready"],
            "readyReason": result["manifest"]["readyReason"],
            "previewMediaRef": result["previewMediaRef"],
            "maskArtifact": {
                "artifactId": result["artifactId"],
                "digest": result["digest"],
                "pageIndexRef": result["pageIndexRef"],
            },
            "framePageIndexRef": result["pageIndexRef"],
        }
        progress = read_progress(artifact_root(base, result["artifactId"])) or {}
        return (
            result["artifactId"],
            result["digest"],
            json.dumps(receipt, ensure_ascii=False),
            json.dumps(progress, ensure_ascii=False),
        )


class BGXAnimateMaskArtifactLoad:
    """Load a previously exported maskArtifact for Mix generation (no re-SAM).

    When generation N/fps differ from the tracking grid, remap with the same
    PTS→index helper as BGXAnimateFrameSequence; spatial resize is nearest-neighbor.
    """

    @classmethod
    def INPUT_TYPES(cls):
        return {
            "required": {
                "artifact_id": ("STRING", {"default": ""}),
                "digest": ("STRING", {"default": ""}),
                "expected_frame_count": ("INT", {"default": 1, "min": 1, "max": 100000}),
            },
            "optional": {
                "target_fps": ("INT", {"default": 0, "min": 0, "max": 120}),
                "frame_times_json": ("STRING", {"default": ""}),
                "out_width": ("INT", {"default": 0, "min": 0, "max": 8192}),
                "out_height": ("INT", {"default": 0, "min": 0, "max": 8192}),
            },
        }

    RETURN_TYPES = ("MASK",)
    RETURN_NAMES = ("mask",)
    FUNCTION = "load"
    CATEGORY = "bgx/wan22-animate"

    def load(
        self,
        artifact_id,
        digest,
        expected_frame_count,
        target_fps=0,
        frame_times_json="",
        out_width=0,
        out_height=0,
    ):
        import folder_paths
        import torch
        from .sequence import map_sequence_indices_from_pts, remap_mask_nearest

        base = Path(folder_paths.get_output_directory())
        frames, manifest = load_masks(base, str(artifact_id), str(digest))
        n = int(expected_frame_count)
        stamps_raw = (frame_times_json or "").strip()
        if stamps_raw:
            stamps = json.loads(stamps_raw)
            fps = int(target_fps) if int(target_fps) > 0 else int(manifest.get("outputFps") or 0)
            if fps <= 0:
                raise ValueError("target_fps is required when remapping masks by PTS")
            indices = map_sequence_indices_from_pts(n, fps, stamps)
            frames = [frames[i] for i in indices]
        elif len(frames) != n:
            raise ValueError(
                f"mask artifact frame count {len(frames)} != expected {n}"
            )
        geom = manifest.get("outputGeometry") or {}
        tw = int(out_width) if int(out_width) > 0 else int(geom.get("width") or 0)
        th = int(out_height) if int(out_height) > 0 else int(geom.get("height") or 0)
        if tw > 0 and th > 0:
            frames = [remap_mask_nearest(f, th, tw) for f in frames]
        batch = torch.stack([torch.from_numpy(f) for f in frames], dim=0).cpu().float()
        return (batch,)


def register_routes():
    """HTTP progress/manifest read for the WASM adapter (not a second task queue)."""
    import folder_paths
    import hashlib
    from aiohttp import web
    from server import PromptServer

    def _base() -> Path:
        return Path(folder_paths.get_output_directory())

    @PromptServer.instance.routes.get("/bgx/wan22-animate/companion/identity")
    async def _companion_identity(request):
        # Seal loaded module identity so the package can detect stale processes.
        here = Path(__file__).resolve()
        digest = hashlib.sha256(here.read_bytes()).hexdigest()
        payload = {
            "version": "0.2.4",
            "module": here.name,
            "moduleDigest": f"sha256:{digest}",
        }
        return web.json_response(payload)

    @PromptServer.instance.routes.get(BASE_HTTP + "/{request_id}/progress")
    async def tracking_progress(request):
        try:
            rid = validate_request_id(request.match_info["request_id"])
            root = artifact_root(_base(), rid)
            payload = read_progress(root)
            if payload is None:
                return web.json_response(
                    {"error": {"code": "NOT_FOUND", "message": "progress not found"}},
                    status=404,
                )
            return web.json_response(payload)
        except ValueError:
            return web.json_response(
                {"error": {"code": "INVALID_REQUEST", "message": "invalid requestId"}},
                status=400,
            )

    @PromptServer.instance.routes.get(BASE_HTTP + "/{request_id}/manifest")
    async def tracking_manifest(request):
        try:
            rid = validate_request_id(request.match_info["request_id"])
            root = artifact_root(_base(), rid)
            path = root / MANIFEST
            if not path.is_file():
                return web.json_response(
                    {"error": {"code": "NOT_FOUND", "message": "manifest not found"}},
                    status=404,
                )
            return web.json_response(json.loads(path.read_text(encoding="utf-8")))
        except ValueError:
            return web.json_response(
                {"error": {"code": "INVALID_REQUEST", "message": "invalid requestId"}},
                status=400,
            )

    @PromptServer.instance.routes.post(BASE_HTTP + "/{request_id}/progress")
    async def tracking_progress_write(request):
        """Adapter may seed queued/loading facts before the Comfy prompt runs."""
        try:
            rid = validate_request_id(request.match_info["request_id"])
            body = await request.json()
            root = artifact_root(_base(), rid)
            mark_running(root)
            payload = write_progress(
                root,
                request_id=rid,
                revision=str(body.get("revision", "1")),
                selection_digest=str(body["selectionDigest"]),
                stage=str(body["stage"]),
                direction=str(body["direction"]),
                total_frames=body.get("totalFrames"),
                processed_frames=body.get("processedFrames"),
                last_completed_time_ms=body.get("lastCompletedTimeMs"),
                covered_ranges=body.get("coveredRanges") or [],
            )
            return web.json_response(payload)
        except (KeyError, ValueError, TypeError):
            return web.json_response(
                {"error": {"code": "INVALID_REQUEST", "message": "invalid progress payload"}},
                status=400,
            )
