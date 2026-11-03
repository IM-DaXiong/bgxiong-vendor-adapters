"""P02 contract helpers for multi-keyframe SAM2 injection (tests/probe only).

Production tracking.py still accepts a single anchor (P12 will adopt these rules).
SPEC section 8 (20261101 video-target-keyframes-v1):
  1. One SAM2 inference state; inject all correction frames for the same obj_id in time order.
  2. clear_old_points clears only the current frame's points.
  3. towardEnd covers the requested end side; towardStart covers the start side.
  4. Correction-frame masks win.
  5. Non-correction frames: nearest correction by temporal distance; ties take the earlier side.
     Forbidden: unconditional OR-union of opposite-direction propagations.
  6. Prefer a verified predictor multi-condition primitive when it already unifies correctly.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Optional, Sequence, Tuple

import numpy as np


@dataclass
class InjectCall:
    frame_idx: int
    obj_id: int
    points: Tuple[Tuple[float, float], ...]
    labels: Tuple[int, ...]
    clear_old_points: bool


@dataclass
class PropagateCall:
    start_frame_idx: Optional[int]
    reverse: bool
    max_frame_num_to_track: Optional[int] = None


@dataclass
class RecordingSam2State:
    """Minimal stand-in for SAM2 inference_state point_inputs_per_obj semantics."""

    num_frames: int
    obj_id: int = 1
    point_inputs_per_frame: Dict[int, List[Tuple[Tuple[float, float], int]]] = field(default_factory=dict)
    inject_calls: List[InjectCall] = field(default_factory=list)
    propagate_calls: List[PropagateCall] = field(default_factory=list)
    state_id: int = 1

    def add_new_points_or_box(
        self,
        frame_idx: int,
        obj_id: int,
        points: Sequence[Sequence[float]],
        labels: Sequence[int],
        clear_old_points: bool = True,
        box=None,
    ) -> None:
        if obj_id != self.obj_id:
            raise ValueError(f'P02 contract uses one obj_id per request; got {obj_id} vs {self.obj_id}')
        if frame_idx < 0 or frame_idx >= self.num_frames:
            raise ValueError(f'frame_idx {frame_idx} outside 0..{self.num_frames - 1}')
        pts = tuple((float(p[0]), float(p[1])) for p in points)
        labs = tuple(int(x) for x in labels)
        if len(pts) != len(labs):
            raise ValueError('points/labels length mismatch')
        self.inject_calls.append(
            InjectCall(frame_idx, obj_id, pts, labs, bool(clear_old_points))
        )
        if clear_old_points or frame_idx not in self.point_inputs_per_frame:
            self.point_inputs_per_frame[frame_idx] = []
        for p, lab in zip(pts, labs):
            self.point_inputs_per_frame[frame_idx].append((p, lab))

    def propagate_in_video(
        self,
        start_frame_idx: Optional[int] = None,
        reverse: bool = False,
        max_frame_num_to_track: Optional[int] = None,
    ) -> PropagateCall:
        call = PropagateCall(start_frame_idx, bool(reverse), max_frame_num_to_track)
        self.propagate_calls.append(call)
        return call


def inject_keyframes_in_time_order(
    state: RecordingSam2State,
    keyframes: Sequence[Tuple[int, Sequence[Sequence[float]], Sequence[int]]],
    obj_id: int = 1,
    clear_old_points: bool = True,
) -> None:
    """Inject many correction frames into one state, sorted by frame index."""
    ordered = sorted(keyframes, key=lambda item: int(item[0]))
    for frame_idx, points, labels in ordered:
        state.add_new_points_or_box(
            frame_idx=int(frame_idx),
            obj_id=obj_id,
            points=points,
            labels=labels,
            clear_old_points=clear_old_points,
        )


def nearest_keyframe_side(
    frame_idx: int,
    keyframes: Sequence[int],
) -> Optional[str]:
    """Return earlier/later/correction for which adjacent correction owns this frame.

    Ties (equal distance) take the earlier side per SPEC section 8 rule 5.
    """
    kfs = sorted(int(k) for k in keyframes)
    if not kfs:
        return None
    if frame_idx in kfs:
        return 'correction'
    earlier = [k for k in kfs if k < frame_idx]
    later = [k for k in kfs if k > frame_idx]
    if earlier and not later:
        return 'earlier'
    if later and not earlier:
        return 'later'
    left = earlier[-1]
    right = later[0]
    d_left = frame_idx - left
    d_right = right - frame_idx
    if d_left < d_right:
        return 'earlier'
    if d_right < d_left:
        return 'later'
    return 'earlier'


def merge_directional_without_or(
    toward_end: Dict[int, np.ndarray],
    toward_start: Dict[int, np.ndarray],
    keyframes: Sequence[int],
    frame_count: int,
    direction: str,
) -> List[np.ndarray]:
    """Merge direction results with nearest-keyframe ownership; never OR opposite sides.

    Coverage walls:
      - towardEnd never invents frames before the earliest correction.
      - towardStart never invents frames after the latest correction.
      - both: outside the keyframe span use the only covering side; inside the span
        pick by nearest correction (tie → earlier / towardStart).
    """
    if direction not in ('towardEnd', 'towardStart', 'both'):
        raise ValueError(f'unknown direction {direction}')
    kfs = sorted(int(k) for k in keyframes)
    if not kfs:
        raise ValueError('keyframes required')
    first_kf, last_kf = kfs[0], kfs[-1]
    out: List[np.ndarray] = []
    for i in range(frame_count):
        side = nearest_keyframe_side(i, kfs)
        if side == 'correction':
            if i in toward_end:
                mask = toward_end[i]
            elif i in toward_start:
                mask = toward_start[i]
            else:
                raise ValueError(f'missing correction mask at frame {i}')
        elif direction == 'towardEnd':
            if i < first_kf:
                raise ValueError(f'towardEnd does not cover uncomputed frame {i}')
            if i not in toward_end:
                raise ValueError(f'towardEnd missing frame {i}')
            mask = toward_end[i]
        elif direction == 'towardStart':
            if i > last_kf:
                raise ValueError(f'towardStart does not cover uncomputed frame {i}')
            if i not in toward_start:
                raise ValueError(f'towardStart missing frame {i}')
            mask = toward_start[i]
        else:  # both
            if i < first_kf:
                choose = 'towardStart'
            elif i > last_kf:
                choose = 'towardEnd'
            elif side == 'earlier':
                choose = 'towardStart'
            elif side == 'later':
                choose = 'towardEnd'
            else:
                raise ValueError(f'cannot assign frame {i}')
            src = toward_start if choose == 'towardStart' else toward_end
            if i not in src:
                raise ValueError(f'{choose} missing frame {i}')
            mask = src[i]
        arr = np.asarray(mask)
        if arr.ndim != 2:
            raise ValueError(f'mask at {i} must be 2-D, got {arr.shape}')
        out.append(arr.astype(np.uint8, copy=False))
    shape = out[0].shape
    for i, m in enumerate(out):
        if m.shape != shape:
            raise ValueError(f'mask shape mismatch at {i}')
    return out


def assert_no_or_merge(
    toward_end: Dict[int, np.ndarray],
    toward_start: Dict[int, np.ndarray],
    merged: Sequence[np.ndarray],
    keyframes: Sequence[int],
) -> None:
    """Guard: merged pixels must come from exactly one side, never OR of both."""
    kfs_sorted = sorted(int(k) for k in keyframes)
    kfs = set(kfs_sorted)
    first_kf, last_kf = kfs_sorted[0], kfs_sorted[-1]
    for i, m in enumerate(merged):
        if i in kfs:
            continue
        side = nearest_keyframe_side(i, keyframes)
        if i < first_kf:
            expect = 'towardStart'
        elif i > last_kf:
            expect = 'towardEnd'
        elif side == 'earlier':
            expect = 'towardStart'
        elif side == 'later':
            expect = 'towardEnd'
        else:
            continue
        te = toward_end.get(i)
        ts = toward_start.get(i)
        if te is None or ts is None:
            continue
        chosen = ts if expect == 'towardStart' else te
        if not np.array_equal(te, ts):
            orred = np.logical_or(te, ts).astype(np.uint8)
            if np.array_equal(m, orred) and (not np.array_equal(m, te)) and (not np.array_equal(m, ts)):
                raise AssertionError(f'frame {i} looks OR-merged across directions')
            if not np.array_equal(m, chosen):
                raise AssertionError(f'frame {i} should take {expect}, not OR/other')
