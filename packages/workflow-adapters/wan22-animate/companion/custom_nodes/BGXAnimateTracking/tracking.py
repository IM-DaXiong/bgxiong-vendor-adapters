"""Multi-keyframe SAM2 mask tracking (video-target-keyframes-v1).

SPEC §8: one inference state per request; clear_old_points current frame only;
nearest-keyframe ownership for non-correction frames; never unconditional OR
of opposite-direction masks. Legacy single-anchor node remains for Mix until
generation consumes a confirmed maskArtifact.
"""

from __future__ import annotations

import json
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple

import numpy as np


def nearest_keyframe_side(frame_idx: int, keyframes: Sequence[int]) -> Optional[str]:
    "Which adjacent correction owns this frame; ties -> earlier (SPEC section 8)."
    kfs = sorted(int(k) for k in keyframes)
    if not kfs:
        return None
    if frame_idx in kfs:
        return "correction"
    earlier = [k for k in kfs if k < frame_idx]
    later = [k for k in kfs if k > frame_idx]
    if earlier and not later:
        return "earlier"
    if later and not earlier:
        return "later"
    left, right = earlier[-1], later[0]
    d_left, d_right = frame_idx - left, right - frame_idx
    if d_left < d_right:
        return "earlier"
    if d_right < d_left:
        return "later"
    return "earlier"


def merge_directional_without_or(
    toward_end: Dict[int, np.ndarray],
    toward_start: Dict[int, np.ndarray],
    keyframes: Sequence[int],
    frame_count: int,
    direction: str,
) -> List[np.ndarray]:
    "Nearest-keyframe ownership; never OR opposite directions (SPEC section 8)."
    if direction not in ("towardEnd", "towardStart", "both"):
        raise ValueError(f"unknown direction {direction}")
    kfs = sorted(int(k) for k in keyframes)
    if not kfs:
        raise ValueError("keyframes required")
    first_kf, last_kf = kfs[0], kfs[-1]
    out: List[np.ndarray] = []
    for i in range(frame_count):
        side = nearest_keyframe_side(i, kfs)
        if side == "correction":
            if i in toward_end:
                mask = toward_end[i]
            elif i in toward_start:
                mask = toward_start[i]
            else:
                raise ValueError(f"missing correction mask at frame {i}")
        elif direction == "towardEnd":
            if i < first_kf:
                raise ValueError(f"towardEnd does not cover uncomputed frame {i}")
            if i not in toward_end:
                raise ValueError(f"towardEnd missing frame {i}")
            mask = toward_end[i]
        elif direction == "towardStart":
            if i > last_kf:
                raise ValueError(f"towardStart does not cover uncomputed frame {i}")
            if i not in toward_start:
                raise ValueError(f"towardStart missing frame {i}")
            mask = toward_start[i]
        else:
            if i < first_kf:
                choose = "towardStart"
            elif i > last_kf:
                choose = "towardEnd"
            elif side == "earlier":
                choose = "towardEnd"
            elif side == "later":
                choose = "towardStart"
            else:
                raise ValueError(f"cannot assign frame {i}")
            src = toward_start if choose == "towardStart" else toward_end
            if i not in src:
                raise ValueError(f"{choose} missing frame {i}")
            mask = src[i]
        arr = np.asarray(mask)
        if arr.ndim != 2:
            raise ValueError(f"mask at {i} must be 2-D, got {arr.shape}")
        out.append(arr.astype(np.uint8, copy=False))
    shape = out[0].shape
    for i, m in enumerate(out):
        if m.shape != shape:
            raise ValueError(f"mask shape mismatch at {i}")
    return out

def anchor_frame_index(frame_time_ms: int, target_fps: int, frame_count: int) -> int:
    if frame_time_ms < 0:
        raise ValueError("frame_time_ms must be non-negative")
    if target_fps <= 0:
        raise ValueError("target_fps must be positive")
    if frame_count <= 0:
        raise ValueError("frame_count must be positive")
    a = (int(frame_time_ms) * int(target_fps)) // 1000
    if a < 0 or a >= int(frame_count):
        raise ValueError(
            f"anchor frame {a} from frame_time_ms={frame_time_ms} is outside 0..{frame_count - 1}"
        )
    return a


def parse_positive_points(coordinates_positive: str) -> np.ndarray:
    raw = coordinates_positive.replace("'", '"')
    data = json.loads(raw)
    if not isinstance(data, list) or not data:
        raise ValueError("coordinates_positive must be a non-empty JSON array")
    pts = []
    for item in data:
        if not isinstance(item, dict) or "x" not in item or "y" not in item:
            raise ValueError("each point must have x and y")
        pts.append((float(item["x"]), float(item["y"])))
    return np.atleast_2d(np.array(pts, dtype=np.float32))


def parse_keyframes_payload(keyframes_json: str) -> List[dict]:
    data = json.loads(keyframes_json.replace("'", '"'))
    if not isinstance(data, list) or not data:
        raise ValueError("keyframes must be a non-empty JSON array")
    out = []
    seen_frames = set()
    for item in data:
        if not isinstance(item, dict):
            raise ValueError("keyframe must be an object")
        frame_index = int(item["frameIndex"])
        if frame_index in seen_frames:
            raise ValueError(f"duplicate frameIndex {frame_index} in keyframes")
        seen_frames.add(frame_index)
        points = item.get("points") or []
        box = item.get("box")
        if not points and box is None:
            raise ValueError("each keyframe needs points or a box")
        out.append(
            {
                "keyframeId": str(item["keyframeId"]),
                "frameIndex": frame_index,
                "timeMs": int(item.get("timeMs", 0)),
                "points": points,
                "box": box,
            }
        )
    out.sort(key=lambda k: k["frameIndex"])
    return out


def keyframe_points_labels(kf: dict) -> Tuple[np.ndarray, np.ndarray, Optional[np.ndarray]]:
    pts = []
    labels = []
    for p in kf.get("points") or []:
        pts.append((float(p["x"]), float(p["y"])))
        lab = p.get("label", 1)
        if lab in ("positive", 1, "1", True):
            labels.append(1)
        elif lab in ("negative", 0, "0", False):
            labels.append(0)
        else:
            labels.append(int(lab))
    box_arr = None
    box = kf.get("box")
    if box is not None:
        x, y, w, h = float(box["x"]), float(box["y"]), float(box["width"]), float(box["height"])
        if w <= 0 or h <= 0:
            raise ValueError("box width/height must be positive")
        # SAM2 box is XYXY
        box_arr = np.array([x, y, x + w, y + h], dtype=np.float32)
    if not pts and box_arr is None:
        raise ValueError("keyframe empty after parse")
    if pts:
        points = np.atleast_2d(np.array(pts, dtype=np.float32))
        labs = np.array(labels, dtype=np.int32)
    else:
        points = np.zeros((0, 2), dtype=np.float32)
        labs = np.zeros((0,), dtype=np.int32)
    return points, labs, box_arr


def merge_bidirectional_masks(
    forward: Dict[int, np.ndarray],
    reverse: Dict[int, np.ndarray],
    frame_count: int,
    anchor: int,
) -> List[np.ndarray]:
    """Legacy single-anchor merge (no OR across directions; partition by anchor)."""
    merged: Dict[int, np.ndarray] = {}
    for idx, mask in forward.items():
        if idx < 0 or idx >= frame_count:
            raise ValueError(f"forward mask frame {idx} outside 0..{frame_count - 1}")
        if idx in merged:
            raise ValueError(f"duplicate forward mask for frame {idx}")
        merged[idx] = mask
    for idx, mask in reverse.items():
        if idx < 0 or idx >= frame_count:
            raise ValueError(f"reverse mask frame {idx} outside 0..{frame_count - 1}")
        if idx == anchor:
            continue
        if idx in merged:
            raise ValueError(f"overlapping non-anchor mask for frame {idx}")
        merged[idx] = mask
    missing = [i for i in range(frame_count) if i not in merged]
    if missing:
        raise ValueError(f"mask coverage incomplete; missing frames {missing[:8]}")
    out = [merged[i] for i in range(frame_count)]
    shape = None
    for i, m in enumerate(out):
        arr = np.asarray(m)
        if arr.ndim != 2:
            raise ValueError(f"mask at frame {i} must be 2-D, got shape {arr.shape}")
        if shape is None:
            shape = arr.shape
        elif arr.shape != shape:
            raise ValueError(f"mask shape mismatch at frame {i}: {arr.shape} vs {shape}")
    return out


class _DefaultSam2Backend:
    """Thin wrapper around a loaded Comfy SAM2MODEL video segmentor."""

    def __init__(self, sam2_model: dict):
        self.sam2_model = sam2_model
        self.model = sam2_model["model"]
        self.device = sam2_model["device"]
        self.dtype = sam2_model["dtype"]
        self.segmentor = sam2_model.get("segmentor")
        if self.segmentor != "video":
            raise ValueError("BGXAnimate tracking requires a video SAM2 segmentor")
        self._state = None

    def run(
        self,
        image,
        points: np.ndarray,
        labels: np.ndarray,
        anchor: int,
        frame_count: int,
    ) -> Tuple[Dict[int, np.ndarray], Dict[int, np.ndarray]]:
        """Legacy single-anchor bidirectional run (one state)."""
        return self.run_multi(
            image,
            keyframes=[
                {
                    "frameIndex": int(anchor),
                    "points": [{"x": float(x), "y": float(y), "label": int(l)} for (x, y), l in zip(points, labels)],
                    "box": None,
                    "keyframeId": "legacy",
                    "timeMs": 0,
                }
            ],
            direction="both",
            frame_count=frame_count,
            obj_id=1,
        )

    def run_multi(
        self,
        image,
        keyframes: Sequence[dict],
        direction: str,
        frame_count: int,
        obj_id: int = 1,
        on_frame: Optional[Callable[[str, int], None]] = None,
    ) -> Tuple[Dict[int, np.ndarray], Dict[int, np.ndarray]]:
        """`on_frame(stage, frame_index)` runs after each propagated frame (towardEnd/towardStart)."""
        import torch
        from contextlib import nullcontext
        import comfy.model_management as mm
        from comfy.utils import common_upscale

        if direction not in ("towardStart", "towardEnd", "both"):
            raise ValueError(f"unknown direction {direction}")
        B, H, W, C = image.shape
        if B != frame_count:
            raise ValueError(f"image batch length {B} must equal frame_count {frame_count}")
        if not keyframes:
            raise ValueError("keyframes required")
        # Reject multi-target: single obj_id only.
        model = self.model
        device = self.device
        dtype = self.dtype
        model_input_image_size = model.image_size
        resized = common_upscale(
            image.movedim(-1, 1),
            model_input_image_size,
            model_input_image_size,
            "bilinear",
            "disabled",
        ).movedim(1, -1)

        try:
            model.to(device)
        except Exception:
            model.model.to(device)

        autocast_condition = not mm.is_device_mps(device)
        ctx = (
            torch.autocast(mm.get_autocast_device(device), dtype=dtype)
            if autocast_condition
            else nullcontext()
        )
        toward_end: Dict[int, np.ndarray] = {}
        toward_start: Dict[int, np.ndarray] = {}
        ordered = sorted(keyframes, key=lambda k: int(k["frameIndex"]))
        try:
            with ctx:
                state = model.init_state(
                    resized.permute(0, 3, 1, 2).contiguous(), H, W, device=device
                )
                self._state = state
                for kf in ordered:
                    frame_idx = int(kf["frameIndex"])
                    if frame_idx < 0 or frame_idx >= frame_count:
                        raise ValueError(f"keyframe frameIndex {frame_idx} out of range")
                    pts, labs, box = keyframe_points_labels(kf)
                    model.add_new_points_or_box(
                        inference_state=state,
                        frame_idx=frame_idx,
                        obj_id=obj_id,
                        points=pts if len(pts) else None,
                        labels=labs if len(labs) else None,
                        clear_old_points=True,  # current frame only
                        box=box,
                    )
                first_kf = int(ordered[0]["frameIndex"])
                last_kf = int(ordered[-1]["frameIndex"])
                if direction in ("towardEnd", "both"):
                    for out_frame_idx, out_obj_ids, out_mask_logits in model.propagate_in_video(
                        state, start_frame_idx=first_kf, reverse=False
                    ):
                        toward_end[int(out_frame_idx)] = self._combine(out_obj_ids, out_mask_logits)
                        if on_frame is not None:
                            on_frame("towardEnd", int(out_frame_idx))
                if direction in ("towardStart", "both"):
                    for out_frame_idx, out_obj_ids, out_mask_logits in model.propagate_in_video(
                        state, start_frame_idx=last_kf, reverse=True
                    ):
                        toward_start[int(out_frame_idx)] = self._combine(out_obj_ids, out_mask_logits)
                        if on_frame is not None:
                            on_frame("towardStart", int(out_frame_idx))
        finally:
            self._cleanup(model)
        return toward_end, toward_start

    @staticmethod
    def _combine(out_obj_ids, out_mask_logits) -> np.ndarray:
        if out_mask_logits.ndim != 4 or out_mask_logits.shape[:2] != (len(out_obj_ids), 1):
            raise ValueError(
                f"SAM2 mask logits must have shape (objects, 1, H, W), got {tuple(out_mask_logits.shape)}"
            )
        # Single-target only: refuse multi-object OR into one mask.
        if len(out_obj_ids) != 1:
            raise ValueError(
                f"expected exactly one tracked object, got obj_ids={list(out_obj_ids)}"
            )
        _, _, h, w = out_mask_logits.shape
        out_mask = (out_mask_logits[0, 0] > 0.0).detach().cpu().numpy()
        return out_mask.astype(np.uint8)

    def _cleanup(self, model) -> None:
        import comfy.model_management as mm

        try:
            if self._state is not None and hasattr(model, "reset_state"):
                model.reset_state(self._state)
        finally:
            self._state = None
            offload = mm.unet_offload_device()
            try:
                model.to(offload)
            except Exception:
                try:
                    model.model.to(offload)
                except Exception:
                    pass
            mm.soft_empty_cache()


def run_bidirectional(
    backend,
    image,
    coordinates_positive: str,
    frame_time_ms: int,
    target_fps: int,
    frame_count: int,
) -> List[np.ndarray]:
    a = anchor_frame_index(frame_time_ms, target_fps, frame_count)
    points = parse_positive_points(coordinates_positive)
    labels = np.ones(len(points), dtype=np.int32)
    forward, reverse = backend.run(image, points, labels, a, frame_count)
    # Legacy path: partition by single anchor (no OR).
    return merge_bidirectional_masks(forward, reverse, frame_count, a)


def coverage_frames(direction: str, keyframe_indices: Sequence[int], frame_count: int) -> List[int]:
    "Frames a request covers: both = all; towardEnd = first keyframe..end; towardStart = start..last keyframe."
    kfs = sorted(int(k) for k in keyframe_indices)
    if not kfs or frame_count <= 0:
        raise ValueError("coverage needs keyframes and frames")
    if direction == "both":
        return list(range(frame_count))
    if direction == "towardEnd":
        return list(range(kfs[0], frame_count))
    if direction == "towardStart":
        return list(range(0, kfs[-1] + 1))
    raise ValueError(f"unknown direction {direction}")


def run_multikeyframe(
    backend,
    image,
    keyframes_json: str,
    direction: str,
    frame_count: int,
    on_frame: Optional[Callable[[str, int], None]] = None,
) -> Tuple[List[np.ndarray], List[int]]:
    keyframes = parse_keyframes_payload(keyframes_json)
    if direction not in ("towardStart", "towardEnd", "both"):
        raise ValueError(f"unsupported direction {direction}")
    kf_indices = [int(k["frameIndex"]) for k in keyframes]
    for idx in kf_indices:
        if idx < 0 or idx >= frame_count:
            raise ValueError(f"keyframe index {idx} outside 0..{frame_count - 1}")
    toward_end, toward_start = backend.run_multi(
        image, keyframes=keyframes, direction=direction, frame_count=frame_count, obj_id=1,
        on_frame=on_frame,
    )
    # For covered range only: merge_directional_without_or requires full span for "both".
    # towardEnd / towardStart may leave walls uncomputed — build only covered indices.
    if direction == "both":
        masks = merge_directional_without_or(
            toward_end, toward_start, kf_indices, frame_count, direction
        )
        return masks, list(range(frame_count))
    if direction == "towardEnd":
        first = min(kf_indices)
        masks = []
        covered = []
        for i in range(first, frame_count):
            if i not in toward_end:
                raise ValueError(f"towardEnd missing frame {i}")
            arr = np.asarray(toward_end[i])
            if arr.ndim != 2:
                raise ValueError(f"mask at {i} must be 2-D")
            masks.append(arr.astype(np.uint8, copy=False))
            covered.append(i)
        return masks, covered
    # towardStart
    last = max(kf_indices)
    masks = []
    covered = []
    for i in range(0, last + 1):
        if i not in toward_start:
            raise ValueError(f"towardStart missing frame {i}")
        arr = np.asarray(toward_start[i])
        if arr.ndim != 2:
            raise ValueError(f"mask at {i} must be 2-D")
        masks.append(arr.astype(np.uint8, copy=False))
        covered.append(i)
    return masks, covered


class BGXAnimateBidirectionalMask:
    """Legacy single-anchor Mix node (kept for 0.1.4-compatible generation prep)."""

    @classmethod
    def INPUT_TYPES(cls):
        return {
            "required": {
                "sam2_model": ("SAM2MODEL",),
                "image": ("IMAGE",),
                "coordinates_positive": ("STRING", {"forceInput": True}),
                "frame_time_ms": ("INT", {"default": 0, "min": 0, "max": 24 * 60 * 60 * 1000}),
                "target_fps": ("INT", {"default": 24, "min": 1, "max": 120}),
                "frame_count": ("INT", {"default": 1, "min": 1, "max": 100000}),
                "keep_model_loaded": ("BOOLEAN", {"default": True}),
            }
        }

    RETURN_TYPES = ("MASK",)
    RETURN_NAMES = ("mask",)
    FUNCTION = "segment"
    CATEGORY = "bgx/wan22-animate"

    def __init__(self):
        self._backend_factory = None

    def segment(
        self,
        sam2_model,
        image,
        coordinates_positive,
        frame_time_ms,
        target_fps,
        frame_count,
        keep_model_loaded=True,
    ):
        import torch

        backend = (
            self._backend_factory(sam2_model)
            if self._backend_factory is not None
            else _DefaultSam2Backend(sam2_model)
        )
        masks = run_bidirectional(
            backend,
            image,
            coordinates_positive,
            int(frame_time_ms),
            int(target_fps),
            int(frame_count),
        )
        out_list = [torch.from_numpy(np.asarray(m).astype(np.float32)) for m in masks]
        mask_batch = torch.stack(out_list, dim=0).cpu().float()
        return (mask_batch,)


class BGXAnimateMultiKeyframeMask:
    """Mask-only multi-keyframe tracking for video-target-keyframes-v1 preview."""

    @classmethod
    def INPUT_TYPES(cls):
        return {
            "required": {
                "sam2_model": ("SAM2MODEL",),
                "image": ("IMAGE",),
                "keyframes_json": ("STRING", {"forceInput": True}),
                "direction": (["towardStart", "towardEnd", "both"],),
                "frame_count": ("INT", {"default": 1, "min": 1, "max": 100000}),
                "keep_model_loaded": ("BOOLEAN", {"default": True}),
                "request_id": ("STRING", {"default": ""}),
                "selection_digest": ("STRING", {"default": ""}),
                "frame_times_json": ("STRING", {"default": "[]"}),
                "span_ms": ("INT", {"default": 1, "min": 1, "max": 24 * 60 * 60 * 1000}),
            }
        }

    RETURN_TYPES = ("MASK", "STRING")
    RETURN_NAMES = ("mask", "covered_indices_json")
    FUNCTION = "segment"
    CATEGORY = "bgx/wan22-animate"

    def __init__(self):
        self._backend_factory = None
        self._progress_factory = None

    def _reporter(self, **kwargs):
        if self._progress_factory is not None:
            return self._progress_factory(**kwargs)
        import folder_paths
        from pathlib import Path
        from .tracking_artifacts import ProgressReporter, artifact_root

        root = artifact_root(Path(folder_paths.get_output_directory()), kwargs["request_id"])
        return ProgressReporter(root, **kwargs)

    def segment(
        self,
        sam2_model,
        image,
        keyframes_json,
        direction,
        frame_count,
        keep_model_loaded=True,
        request_id="",
        selection_digest="",
        frame_times_json="[]",
        span_ms=1,
    ):
        import torch

        backend = (
            self._backend_factory(sam2_model)
            if self._backend_factory is not None
            else _DefaultSam2Backend(sam2_model)
        )
        B = int(image.shape[0])
        if B != int(frame_count):
            raise ValueError(f"image batch {B} != frame_count {frame_count}")
        frame_times = json.loads(frame_times_json)
        if not isinstance(frame_times, list) or len(frame_times) != int(frame_count):
            raise ValueError("frame_times_json must give one time per frame")
        keyframe_indices = [k["frameIndex"] for k in parse_keyframes_payload(keyframes_json)]
        reporter = self._reporter(
            request_id=str(request_id),
            selection_digest=str(selection_digest),
            direction=str(direction),
            frame_times=frame_times,
            coverage=coverage_frames(str(direction), keyframe_indices, int(frame_count)),
            span_ms=int(span_ms),
        )
        reporter.stage("loading")
        masks, covered = run_multikeyframe(
            backend, image, keyframes_json, str(direction), int(frame_count), on_frame=reporter.frame
        )
        reporter.flush()
        # Full-length batch: uncomputed frames are empty masks (not OR-filled).
        h, w = np.asarray(masks[0]).shape
        full = [np.zeros((h, w), dtype=np.float32) for _ in range(int(frame_count))]
        for m, idx in zip(masks, covered):
            arr = np.asarray(m).astype(np.float32)
            if arr.shape != (h, w):
                raise ValueError(f"mask shape mismatch at {idx}")
            full[idx] = arr
        mask_batch = torch.stack([torch.from_numpy(m) for m in full], dim=0).cpu().float()
        return (mask_batch, json.dumps(covered))
