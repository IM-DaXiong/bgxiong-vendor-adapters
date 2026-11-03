"""Offline PTS/spatial mapping for BGXAnimate sequence (no GPU)."""
from __future__ import annotations
import sys
from pathlib import Path
import numpy as np

ROOT = Path(__file__).resolve().parents[1] / "custom_nodes" / "BGXAnimateTracking"
sys.path.insert(0, str(ROOT))

from sequence import (  # noqa: E402
    map_sequence_indices,
    map_sequence_indices_from_pts,
    remap_mask_nearest,
    source_index_for_target_ms,
    target_time_ms,
)

def test_24_to_24_identity():
    stamps = [i * 1000 // 24 for i in range(24)]
    assert map_sequence_indices_from_pts(24, 24, stamps) == list(range(24))

def test_30_to_24_uses_last_not_later():
    stamps = [i * 1000 // 30 for i in range(30)]
    idx = map_sequence_indices_from_pts(24, 24, stamps)
    assert len(idx) == 24 and idx[0] == 0 and stamps[1] == 33
    assert source_index_for_target_ms(41, stamps) == 1
    assert idx[1] == 1

def test_non_integer_fps_synthetic():
    idx = map_sequence_indices(12, 12, 23.976, 24, None)
    assert len(idx) == 12 and idx[0] == 0 and max(idx) < 12

def test_vfr_nonzero_pts_before_origin_rejected():
    stamps = [83, 125, 166, 208, 250, 291, 333, 375]
    try:
        map_sequence_indices_from_pts(1, 24, stamps)
        raise AssertionError("expected ValueError")
    except ValueError:
        pass

def test_nonzero_selection_normalized_stamps():
    stamps = [0, 42, 83, 125, 166]
    assert map_sequence_indices_from_pts(3, 24, stamps) == [0, 0, 2]  # 41ms still maps to pts0

def test_past_end_rejected():
    stamps = [0, 40, 80]
    try:
        source_index_for_target_ms(120, stamps)
        raise AssertionError("expected ValueError")
    except ValueError:
        pass

def test_same_indices_picture_and_mask():
    stamps = [i * 1000 // 24 for i in range(48)]
    idx = map_sequence_indices_from_pts(24, 24, stamps)
    picture = list(range(48)); masks = list(range(48))
    assert [picture[i] for i in idx] == [masks[i] for i in idx]

def test_nearest_neighbor_1920x1080_to_1280x720():
    src = np.zeros((1080, 1920), dtype=np.float32)
    src[100:200, 200:400] = 7.0
    out = remap_mask_nearest(src, 720, 1280)
    assert out.shape == (720, 1280)
    uniq = set(np.unique(out).tolist())
    assert uniq <= {0.0, 7.0} and 7.0 in uniq

def test_target_time_ms_rational():
    assert target_time_ms(0, 24) == 0
    assert target_time_ms(1, 24) == 41
    assert target_time_ms(24, 24) == 1000

if __name__ == "__main__":
    for name, fn in list(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print("PASS", name)
    print("ALL_OK")