"""PTS-based time remapping shared by generation frames and masks.

Target sample j at time floor(j * 1000 / fps) ms (integer ms) maps to the last
source sample whose presentation timestamp is not later than that moment.
Half-open source span: times beyond the last sample's exclusive end are rejected.
"""

from __future__ import annotations

import json
import math
from fractions import Fraction
from typing import List, Optional, Sequence, Tuple


def _as_int_ms_list(timestamps_ms: Sequence[object]) -> List[int]:
    out: List[int] = []
    for item in timestamps_ms:
        if isinstance(item, bool):
            raise ValueError("timestampsMs entries must be integers")
        if isinstance(item, int):
            out.append(item)
        elif isinstance(item, float) and item.is_integer():
            out.append(int(item))
        elif isinstance(item, str):
            out.append(int(item))
        else:
            raise ValueError("timestampsMs entries must be integers")
    if not out:
        raise ValueError("timestampsMs must be non-empty")
    for a, b in zip(out, out[1:]):
        if b < a:
            raise ValueError("timestampsMs must be non-decreasing")
    return out


def source_index_for_target_ms(target_ms: int, timestamps_ms: Sequence[int]) -> int:
    """Last source index with pts <= target_ms. Same pts keeps the last index."""
    if target_ms < 0:
        raise ValueError("target_ms must be non-negative")
    stamps = list(timestamps_ms)
    if not stamps:
        raise ValueError("timestampsMs must be non-empty")
    if target_ms < stamps[0]:
        raise ValueError(
            f"target_ms {target_ms} is before the first source sample {stamps[0]}"
        )
    # Exclusive end of the last sample: mid-point to next would require duration;
    # plan: last frame only usable within its legal presentation interval — use
    # last→+∞ only up to last+(last-prev) when len>1, else last+1.
    if len(stamps) == 1:
        end_exclusive = stamps[0] + 1
    else:
        step = max(1, stamps[-1] - stamps[-2])
        end_exclusive = stamps[-1] + step
    if target_ms >= end_exclusive:
        raise ValueError(
            f"target_ms {target_ms} is past the source presentation end {end_exclusive}"
        )
    chosen = 0
    for i, pts in enumerate(stamps):
        if pts <= target_ms:
            chosen = i
        else:
            break
    return chosen


def target_time_ms(j: int, fps: int) -> int:
    if j < 0:
        raise ValueError("output frame index must be non-negative")
    if fps <= 0:
        raise ValueError("fps must be positive")
    # Rational j*1000/fps, then floor to integer ms for comparison.
    return int(Fraction(j * 1000, fps))


def map_sequence_indices_from_pts(
    frame_count: int,
    fps: int,
    timestamps_ms: Sequence[object],
) -> List[int]:
    if frame_count <= 0:
        raise ValueError("frame_count must be positive")
    stamps = _as_int_ms_list(timestamps_ms)
    indices: List[int] = []
    for j in range(frame_count):
        t = target_time_ms(j, fps)
        indices.append(source_index_for_target_ms(t, stamps))
    return indices


def map_sequence_indices(
    frame_count: int,
    source_len: int,
    source_fps: float,
    target_fps: float,
    timestamps_ms: Optional[Sequence[object]] = None,
) -> List[int]:
    """Prefer real PTS; synthesize uniform stamps from source_fps when absent."""
    if timestamps_ms is not None:
        indices = map_sequence_indices_from_pts(
            frame_count, int(target_fps), timestamps_ms
        )
    else:
        if source_len <= 0:
            raise ValueError("source sequence is empty")
        if not (isinstance(source_fps, (int, float)) and source_fps > 0):
            raise ValueError("source_fps must be a positive number")
        if not (isinstance(target_fps, (int, float)) and target_fps > 0):
            raise ValueError("target_fps must be a positive number")
        # Synthetic PTS at i * 1000 / source_fps (floor).
        synthetic = [
            int(math.floor(i * 1000.0 / float(source_fps))) for i in range(source_len)
        ]
        indices = map_sequence_indices_from_pts(
            frame_count, int(target_fps), synthetic
        )
    for j, idx in enumerate(indices):
        if idx < 0 or idx >= source_len:
            raise ValueError(
                f"mapped source index {idx} for output frame {j} is outside 0..{source_len - 1}; refusing to pad"
            )
    return indices


def remap_mask_nearest(
    mask,
    out_h: int,
    out_w: int,
):
    """Nearest-neighbor resize that preserves discrete mask labels."""
    import numpy as np

    arr = np.asarray(mask)
    if arr.ndim != 2:
        raise ValueError(f"mask must be 2-D, got {arr.shape}")
    if out_h <= 0 or out_w <= 0:
        raise ValueError("output size must be positive")
    src_h, src_w = arr.shape
    if src_h == out_h and src_w == out_w:
        return arr.copy()
    # Map each output pixel to nearest source sample (round-half-up via floor(x+0.5)).
    ys = np.minimum(
        src_h - 1,
        np.floor((np.arange(out_h) + 0.5) * src_h / out_h).astype(np.int64),
    )
    xs = np.minimum(
        src_w - 1,
        np.floor((np.arange(out_w) + 0.5) * src_w / out_w).astype(np.int64),
    )
    return arr[ys][:, xs]


def select_by_indices(batch, indices: Sequence[int]):
    """Apply the same index array to an IMAGE/MASK batch."""
    import torch

    if batch is None or getattr(batch, "ndim", 0) < 3:
        raise ValueError("batch must be a rank-3+ tensor")
    source_len = int(batch.shape[0])
    for idx in indices:
        if idx < 0 or idx >= source_len:
            raise ValueError(f"index {idx} outside 0..{source_len - 1}")
    if list(indices) == list(range(source_len)) and source_len == len(indices):
        return batch
    selected = [batch[i] for i in indices]
    return torch.stack(selected, dim=0)


class BGXAnimateFrameSequence:
    """Build exactly N frames on the target timeline; optional PTS JSON overrides fps guess."""

    @classmethod
    def INPUT_TYPES(cls):
        return {
            "required": {
                "images": ("IMAGE",),
                "source_fps": ("FLOAT", {"default": 24.0, "min": 0.001, "max": 240.0, "step": 0.001}),
                "target_fps": ("INT", {"default": 24, "min": 1, "max": 120}),
                "frame_count": ("INT", {"default": 1, "min": 1, "max": 100000}),
            },
            "optional": {
                "frame_times_json": ("STRING", {"default": ""}),
            },
        }

    RETURN_TYPES = ("IMAGE",)
    RETURN_NAMES = ("images",)
    FUNCTION = "build"
    CATEGORY = "bgx/wan22-animate"

    def build(self, images, source_fps, target_fps, frame_count, frame_times_json=""):
        if images is None or getattr(images, "ndim", 0) != 4:
            raise ValueError("images must be a 4-D IMAGE batch")
        source_len = int(images.shape[0])
        stamps = None
        raw = (frame_times_json or "").strip()
        if raw:
            parsed = json.loads(raw)
            if not isinstance(parsed, list):
                raise ValueError("frame_times_json must be a JSON array")
            stamps = parsed
        indices = map_sequence_indices(
            int(frame_count),
            source_len,
            float(source_fps),
            float(target_fps),
            stamps,
        )
        return (select_by_indices(images, indices),)