"""Behavior tests for BGXAnimate tracking companion (no real SAM2 weights)."""

from __future__ import annotations

import importlib.util
import sys
import unittest
import hashlib
import json
import tempfile
from pathlib import Path
from typing import Dict, List, Tuple

import numpy as np
import torch

ROOT = Path(__file__).resolve().parents[1] / "custom_nodes" / "BGXAnimateTracking"
sys.path.insert(0, str(ROOT.parent))


def _load(name: str):
    path = ROOT / f"{name}.py"
    spec = importlib.util.spec_from_file_location(f"bgx_animate_{name}", path)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(mod)
    return mod


sequence = _load("sequence")
tracking = _load("tracking")
reference = _load("reference")

from multikeyframe_contract import (
    RecordingSam2State,
    assert_no_or_merge,
    inject_keyframes_in_time_order,
    merge_directional_without_or,
    nearest_keyframe_side,
)


class ReferencePreparationTests(unittest.TestCase):
    def test_aspect_ratios_preserve_all_edges_and_original(self):
        from PIL import Image, ImageDraw
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for sw, sh, tw, th in [(941, 1672, 1280, 720), (800, 400, 320, 640),
                                   (400, 400, 640, 360), (320, 180, 320, 180),
                                   (1, 800, 320, 180), (800, 1, 180, 320)]:
                with self.subTest(size=(sw, sh, tw, th)):
                    source = Image.new("RGB", (sw, sh), (20, 40, 60))
                    draw = ImageDraw.Draw(source)
                    draw.rectangle((0, 0, sw - 1, max(0, sh // 5)), fill=(255, 0, 0))
                    draw.rectangle((0, sh * 4 // 5, sw - 1, sh - 1), fill=(0, 0, 255))
                    path = root / "original.png"
                    source.save(path)
                    before = hashlib.sha256(path.read_bytes()).digest()
                    result = reference.prepare_reference(root, {"image": path.name, "width": tw, "height": th})
                    self.assertEqual(before, hashlib.sha256(path.read_bytes()).digest())
                    with Image.open(root / result["image"]) as image:
                        self.assertEqual(image.size, (tw, th))
                        c = result["content"]
                        self.assertLessEqual(c["x"] + c["width"], tw)
                        self.assertLessEqual(c["y"] + c["height"], th)
                        # Top/head and bottom/feet survive; no centre crop of either axis.
                        if sh > 1:
                            self.assertGreater(image.getpixel((c["x"], c["y"]))[0], 200)
                            self.assertGreater(image.getpixel((c["x"], c["y"] + c["height"] - 1))[2], 200)
                        if c["x"] > 0 or c["y"] > 0:
                            self.assertEqual(image.getpixel((0, 0)), reference.BACKGROUND)

    def test_exif_orientation_and_transparency(self):
        from PIL import Image
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            image = Image.new("RGB", (20, 40), "red")
            exif = image.getexif()
            exif[274] = 6
            image.save(root / "rotated.jpg", exif=exif)
            result = reference.prepare_reference(root, {"image": "rotated.jpg", "width": 80, "height": 40})
            self.assertEqual((result["sourceWidth"], result["sourceHeight"]), (40, 20))
            image = Image.new("RGBA", (20, 20), (0, 255, 0, 0))
            image.putpixel((10, 10), (255, 0, 0, 255))
            image.save(root / "transparent.png")
            result = reference.prepare_reference(root, {"image": "transparent.png", "width": 20, "height": 20})
            with Image.open(root / result["image"]) as prepared:
                self.assertEqual(prepared.getpixel((0, 0)), reference.BACKGROUND)
                self.assertEqual(prepared.getpixel((10, 10)), (255, 0, 0))

    def test_invalid_paths_dimensions_and_corrupt_images_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for image in ["../outside.png", "/outside.png", "C:/outside.png", "a\\b.png", "a//b.png"]:
                with self.subTest(image=image), self.assertRaises(ValueError):
                    reference.prepare_reference(root, {"image": image, "width": 320, "height": 180})
            for width in [0, -1, True, 1.5]:
                with self.subTest(width=width), self.assertRaises(ValueError):
                    reference.prepare_reference(root, {"image": "x.png", "width": width, "height": 180})
            (root / "broken.png").write_bytes(b"not an image")
            with self.assertRaises(reference.UnidentifiedImageError):
                reference.prepare_reference(root, {"image": "broken.png", "width": 320, "height": 180})
            self.assertFalse((root / "bgx-wan22-reference").exists())

    def test_http_contract(self):
        import asyncio
        from types import SimpleNamespace
        from unittest.mock import patch
        from aiohttp import web
        from aiohttp.test_utils import TestClient, TestServer
        from PIL import Image

        async def exercise(root):
            routes = web.RouteTableDef()
            with patch.dict(sys.modules, {
                    "server": SimpleNamespace(PromptServer=SimpleNamespace(instance=SimpleNamespace(routes=routes))),
                    "folder_paths": SimpleNamespace(get_input_directory=lambda: str(root))}):
                reference.register_routes()
                app = web.Application()
                app.add_routes(routes)
                async with TestClient(TestServer(app)) as client:
                    response = await client.get(reference.BASE_PATH)
                    self.assertEqual((await response.json())["policy"], reference.POLICY)
                    response = await client.post(reference.BASE_PATH, json={"image": "ref.png", "width": 320, "height": 180})
                    self.assertEqual(response.status, 200)
                    result = await response.json()
                    with Image.open(root / result["image"]) as prepared:
                        self.assertEqual(prepared.size, (320, 180))
                    response = await client.post(reference.BASE_PATH, json={"image": "../ref.png", "width": 320, "height": 180})
                    self.assertEqual(response.status, 400)
                    response = await client.post(reference.BASE_PATH, data="invalid-json", headers={"Content-Type": "application/json"})
                    self.assertEqual(response.status, 400)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            Image.new("RGB", (94, 167), "red").save(root / "ref.png")
            asyncio.run(exercise(root))


class FakeBackend:
    """Records propagation calls and returns deterministic per-frame masks."""

    def __init__(self, height: int = 4, width: int = 4, empty_at=None, bad_shape_at=None, drop_frames=None):
        self.height = height
        self.width = width
        self.empty_at = set(empty_at or [])
        self.bad_shape_at = set(bad_shape_at or [])
        self.drop_frames = set(drop_frames or [])
        self.calls: List[Tuple[int, bool]] = []
        self.states: List[int] = []
        self._state_id = 0

    def run(self, image, points, labels, anchor, frame_count):
        self._state_id += 1
        state = self._state_id
        self.states.append(state)
        B = int(getattr(image, "shape", [frame_count])[0])
        if B != frame_count:
            raise ValueError("image/frame_count mismatch")
        # Forward a..N-1
        self.calls.append((anchor, False))
        forward = {}
        for i in range(anchor, frame_count):
            if i in self.drop_frames:
                continue
            forward[i] = self._mask(i)
        # Reverse a..0
        self.calls.append((anchor, True))
        reverse = {}
        for i in range(anchor, -1, -1):
            if i in self.drop_frames:
                continue
            reverse[i] = self._mask(i)
        return forward, reverse

    def _mask(self, idx: int) -> np.ndarray:
        if idx in self.bad_shape_at:
            return np.zeros((self.height + 1, self.width), dtype=np.uint8)
        if idx in self.empty_at:
            return np.zeros((self.height, self.width), dtype=np.uint8)
        m = np.zeros((self.height, self.width), dtype=np.uint8)
        m[idx % self.height, idx % self.width] = 1
        return m


class SequenceTests(unittest.TestCase):
    def test_source24_target16(self):
        # j=0..15 at 16fps from 24fps source: floor(j*24/16)=floor(j*1.5)
        idxs = sequence.map_sequence_indices(16, 24, 24, 16)
        self.assertEqual(idxs[0], 0)
        self.assertEqual(idxs[1], 1)
        self.assertEqual(idxs[2], 3)
        self.assertEqual(len(idxs), 16)
        self.assertLess(max(idxs), 24)

    def test_source16_target24(self):
        idxs = sequence.map_sequence_indices(24, 48, 16, 24)
        self.assertEqual(idxs[0], 0)
        self.assertEqual(idxs[1], 0)
        self.assertEqual(idxs[2], 1)
        self.assertEqual(idxs[3], 2)

    def test_non_integer_fps(self):
        idxs = sequence.map_sequence_indices(5, 20, 23.976, 16)
        self.assertEqual(len(idxs), 5)
        self.assertTrue(all(0 <= i < 20 for i in idxs))

    def test_out_of_range_raises_no_pad(self):
        with self.assertRaises(ValueError):
            sequence.map_sequence_indices(10, 5, 24, 24)


class TrackingTests(unittest.TestCase):
    def test_sam2_logits_to_bidirectional_node_mask_batch(self):
        # Exercise the production conversion, not pre-shaped fake 2-D masks.
        # Include H=1: an unrestricted squeeze would lose a spatial axis.
        for height, width in [(3, 5), (1, 5), (3, 1)]:
            logits = torch.full((1, 1, height, width), -1.0)
            logits[0, 0, 0, 0] = 1.0
            mask = tracking._DefaultSam2Backend._combine([1], logits)
            expected = np.zeros((height, width), dtype=np.uint8)
            expected[0, 0] = 1
            np.testing.assert_array_equal(mask, expected)
            empty = tracking._DefaultSam2Backend._combine([1], torch.full_like(logits, -1))

            class ConvertedBackend:
                def run(self, image, points, labels, anchor, frame_count):
                    return {1: mask, 2: empty}, {1: mask, 0: mask}

            node = tracking.BGXAnimateBidirectionalMask()
            node._backend_factory = lambda _: ConvertedBackend()
            (batch,) = node.segment({}, torch.zeros((3, height, width, 3)),
                                    '[{"x":0,"y":0}]', 1000, 1, 3)
            self.assertEqual(tuple(batch.shape), (3, height, width))
            np.testing.assert_array_equal(batch[0].numpy(), expected)
            np.testing.assert_array_equal(batch[1].numpy(), expected)
            self.assertEqual(batch[2].sum().item(), 0)

    def test_sam2_logits_invalid_channels_or_objects_rejected(self):
        for shape in [(2, 2, 3, 5), (2, 1, 3, 5), (2, 3, 5)]:
            with self.assertRaisesRegex(ValueError, "SAM2 mask logits|exactly one tracked object"):
                tracking._DefaultSam2Backend._combine([1, 2], torch.zeros(shape))

    def test_nonzero_anchor_bidirectional_order_and_dedupe(self):
        backend = FakeBackend()
        n = 8
        a = tracking.anchor_frame_index(1500, 4, 8)
        self.assertEqual(a, 6)
        image = np.zeros((8, 4, 4, 3), dtype=np.float32)
        masks = tracking.run_bidirectional(
            backend,
            image,
            '[{"x":1,"y":1}]',
            1500,
            4,
            8,
        )
        self.assertEqual(len(masks), 8)
        self.assertEqual(backend.calls, [(6, False), (6, True)])
        # Each frame once
        for i, m in enumerate(masks):
            self.assertEqual(m.shape, (4, 4))

    def test_anchor_zero_and_last(self):
        for a_ms, fps, n, expect in [(0, 24, 5, 0), (4000, 1, 5, 4)]:
            self.assertEqual(tracking.anchor_frame_index(a_ms, fps, n), expect)
            backend = FakeBackend()
            image = np.zeros((n, 4, 4, 3), dtype=np.float32)
            masks = tracking.run_bidirectional(backend, image, '[{"x":0,"y":0}]', a_ms, fps, n)
            self.assertEqual(len(masks), n)
            self.assertEqual(backend.calls[0], (expect, False))
            self.assertEqual(backend.calls[1], (expect, True))

    def test_empty_mask_retained(self):
        backend = FakeBackend(empty_at={2, 3})
        image = np.zeros((5, 4, 4, 3), dtype=np.float32)
        masks = tracking.run_bidirectional(backend, image, '[{"x":0,"y":0}]', 0, 24, 5)
        self.assertEqual(masks[2].sum(), 0)
        self.assertEqual(masks[3].sum(), 0)
        self.assertGreater(masks[0].sum(), 0)

    def test_missing_frame_fails(self):
        backend = FakeBackend(drop_frames={1})
        image = np.zeros((4, 4, 4, 3), dtype=np.float32)
        with self.assertRaises(ValueError):
            tracking.run_bidirectional(backend, image, '[{"x":0,"y":0}]', 0, 24, 4)

    def test_bad_shape_fails(self):
        backend = FakeBackend(bad_shape_at={2})
        image = np.zeros((4, 4, 4, 3), dtype=np.float32)
        with self.assertRaises(ValueError):
            tracking.run_bidirectional(backend, image, '[{"x":0,"y":0}]', 0, 24, 4)

    def test_anchor_out_of_range(self):
        with self.assertRaises(ValueError):
            tracking.anchor_frame_index(5000, 24, 3)

    def test_requests_isolated_state(self):
        backend = FakeBackend()
        image = np.zeros((3, 4, 4, 3), dtype=np.float32)
        tracking.run_bidirectional(backend, image, '[{"x":0,"y":0}]', 0, 24, 3)
        tracking.run_bidirectional(backend, image, '[{"x":1,"y":1}]', 1000, 1, 3)
        self.assertEqual(backend.states, [1, 2])
        self.assertNotEqual(backend.states[0], backend.states[1])




class MultiKeyframeContractTests(unittest.TestCase):
    """P02: describe/verify multi-condition injection contract (no production API change).

    FakeBackend remains for single-anchor regression only. These tests encode SPEC section 8
    rules that P12 must implement against a real SAM2 state; they do not claim
    production tracking.py already performs multi-keyframe injection.
    """

    def test_multi_keyframe_injection_one_state_time_order(self):
        state = RecordingSam2State(num_frames=9, obj_id=1)
        # Deliberately unsorted input; injector must sort by frame index.
        inject_keyframes_in_time_order(
            state,
            [
                (6, [(40.0, 20.0)], [1]),
                (2, [(10.0, 12.0)], [1]),
                (4, [(22.0, 18.0)], [1]),
            ],
            obj_id=1,
            clear_old_points=True,
        )
        self.assertEqual([c.frame_idx for c in state.inject_calls], [2, 4, 6])
        self.assertEqual({c.obj_id for c in state.inject_calls}, {1})
        self.assertEqual(len(state.point_inputs_per_frame), 3)
        self.assertEqual(state.point_inputs_per_frame[2][0][0], (10.0, 12.0))
        self.assertEqual(state.point_inputs_per_frame[6][0][0], (40.0, 20.0))

    def test_clear_old_points_per_frame_only(self):
        state = RecordingSam2State(num_frames=5, obj_id=1)
        state.add_new_points_or_box(1, 1, [(5.0, 5.0)], [1], clear_old_points=True)
        state.add_new_points_or_box(1, 1, [(6.0, 6.0), (7.0, 7.0)], [1, 0], clear_old_points=True)
        state.add_new_points_or_box(3, 1, [(30.0, 10.0)], [1], clear_old_points=True)
        self.assertEqual(len(state.point_inputs_per_frame[1]), 2)
        self.assertEqual(state.point_inputs_per_frame[1][0][0], (6.0, 6.0))
        self.assertEqual(state.point_inputs_per_frame[3][0][0], (30.0, 10.0))
        # Re-clear frame 1 must not wipe frame 3.
        state.add_new_points_or_box(1, 1, [(8.0, 8.0)], [1], clear_old_points=True)
        self.assertEqual(state.point_inputs_per_frame[1][0][0], (8.0, 8.0))
        self.assertEqual(state.point_inputs_per_frame[3][0][0], (30.0, 10.0))
        self.assertTrue(all(c.clear_old_points for c in state.inject_calls))

    def test_nearest_keyframe_tie_takes_earlier(self):
        self.assertEqual(nearest_keyframe_side(4, [2, 6]), "earlier")
        self.assertEqual(nearest_keyframe_side(3, [2, 6]), "earlier")
        self.assertEqual(nearest_keyframe_side(5, [2, 6]), "later")
        self.assertEqual(nearest_keyframe_side(2, [2, 6]), "correction")

    def test_toward_end_and_start_coverage_without_or_merge(self):
        h = w = 4
        keyframes = [2, 6]
        toward_end = {}
        toward_start = {}
        for i in range(8):
            te = np.zeros((h, w), dtype=np.uint8)
            ts = np.zeros((h, w), dtype=np.uint8)
            te[0, i % w] = 1
            ts[1, i % w] = 1
            toward_end[i] = te
            toward_start[i] = ts

        both = merge_directional_without_or(
            toward_end, toward_start, keyframes, frame_count=8, direction="both"
        )
        assert_no_or_merge(toward_end, toward_start, both, keyframes)
        # Before first keyframe → towardStart only; after last → towardEnd only.
        np.testing.assert_array_equal(both[0], toward_start[0])
        np.testing.assert_array_equal(both[1], toward_start[1])
        np.testing.assert_array_equal(both[7], toward_end[7])
        # Inside span: nearer/tie→earlier uses towardStart; nearer later uses towardEnd.
        np.testing.assert_array_equal(both[3], toward_start[3])
        np.testing.assert_array_equal(both[4], toward_start[4])
        np.testing.assert_array_equal(both[5], toward_end[5])
        np.testing.assert_array_equal(both[2], toward_end[2])

        # towardEnd cannot invent frames before the earliest correction.
        with self.assertRaises(ValueError):
            merge_directional_without_or(
                toward_end, toward_start, keyframes, frame_count=8, direction="towardEnd"
            )
        # When the earliest correction is frame 0, towardEnd covers the full span.
        end_only = merge_directional_without_or(
            toward_end, toward_start, [0, 6], frame_count=8, direction="towardEnd"
        )
        self.assertEqual(len(end_only), 8)
        np.testing.assert_array_equal(end_only[7], toward_end[7])

        # towardStart with last_kf=6 covers 0..6; requesting frame_count=8 must fail.
        with self.assertRaises(ValueError):
            merge_directional_without_or(
                toward_end, toward_start, keyframes, frame_count=8, direction="towardStart"
            )
        start_only = merge_directional_without_or(
            toward_end, toward_start, [2, 7], frame_count=8, direction="towardStart"
        )
        np.testing.assert_array_equal(start_only[0], toward_start[0])
        # Correction frames prefer toward_end when present (unified multi-condition mask).
        np.testing.assert_array_equal(start_only[7], toward_end[7])
        np.testing.assert_array_equal(start_only[3], toward_start[3])

    def test_or_merge_guard_detects_forbidden_union(self):
        h = w = 3
        keyframes = [1, 3]
        toward_end = {
            0: np.zeros((h, w), np.uint8),
            1: np.zeros((h, w), np.uint8),
            2: np.array([[1, 0, 0], [0, 0, 0], [0, 0, 0]], np.uint8),
            3: np.zeros((h, w), np.uint8),
        }
        toward_start = {
            0: np.zeros((h, w), np.uint8),
            1: np.zeros((h, w), np.uint8),
            2: np.array([[0, 0, 0], [0, 0, 1], [0, 0, 0]], np.uint8),
            3: np.zeros((h, w), np.uint8),
        }
        bad = [
            toward_end[0],
            toward_end[1],
            np.logical_or(toward_end[2], toward_start[2]).astype(np.uint8),
            toward_end[3],
        ]
        with self.assertRaises(AssertionError):
            assert_no_or_merge(toward_end, toward_start, bad, keyframes)

    def test_directional_propagate_calls_recorded_separately(self):
        state = RecordingSam2State(num_frames=8, obj_id=1)
        inject_keyframes_in_time_order(
            state,
            [(3, [(1.0, 1.0)], [1]), (5, [(2.0, 2.0)], [1])],
        )
        state.propagate_in_video(start_frame_idx=3, reverse=False)
        state.propagate_in_video(start_frame_idx=5, reverse=True)
        self.assertEqual(
            [(c.start_frame_idx, c.reverse) for c in state.propagate_calls],
            [(3, False), (5, True)],
        )





class MultiKeyframeProductionTests(unittest.TestCase):
    """Production tracking.run_multikeyframe behavior (not FakeBackend-as-SAM)."""

    def test_direction_both_uses_nearest_keyframe_no_or(self):
        from multikeyframe_contract import assert_no_or_merge, nearest_keyframe_side

        class MultiBackend:
            def run_multi(self, image, keyframes, direction, frame_count, obj_id=1, on_frame=None):
                self.keyframes = list(keyframes)
                self.direction = direction
                self.obj_id = obj_id
                te, ts = {}, {}
                # Distinct masks per direction so OR would differ from either side.
                for i in range(frame_count):
                    a = np.zeros((4, 4), dtype=np.uint8)
                    a[0, 0] = 1
                    b = np.zeros((4, 4), dtype=np.uint8)
                    b[3, 3] = 1
                    te[i] = a
                    ts[i] = b
                return te, ts

        backend = MultiBackend()
        image = np.zeros((9, 4, 4, 3), dtype=np.float32)
        payload = json.dumps([
            {"keyframeId": "kf0", "frameIndex": 2, "timeMs": 200, "points": [{"x": 1, "y": 1, "label": 1}]},
            {"keyframeId": "kf1", "frameIndex": 6, "timeMs": 600, "points": [{"x": 2, "y": 2, "label": 1}]},
        ])
        masks, covered = tracking.run_multikeyframe(backend, image, payload, "both", 9)
        self.assertEqual(covered, list(range(9)))
        self.assertEqual(len(masks), 9)
        self.assertEqual(backend.obj_id, 1)
        self.assertEqual([k["frameIndex"] for k in backend.keyframes], [2, 6])
        # Rebuild side maps as backend produced and assert no OR.
        te = {i: np.zeros((4, 4), dtype=np.uint8) for i in range(9)}
        ts = {i: np.zeros((4, 4), dtype=np.uint8) for i in range(9)}
        for i in range(9):
            te[i][0, 0] = 1
            ts[i][3, 3] = 1
        assert_no_or_merge(te, ts, masks, [2, 6])
        self.assertEqual(nearest_keyframe_side(4, [2, 6]), "earlier")

    def test_empty_and_conflict_and_multi_target_reject(self):
        with self.assertRaises(ValueError):
            tracking.parse_keyframes_payload("[]")
        with self.assertRaises(ValueError):
            tracking.parse_keyframes_payload(json.dumps([
                {"keyframeId": "a", "frameIndex": 1, "points": [{"x": 0, "y": 0, "label": 1}]},
                {"keyframeId": "b", "frameIndex": 1, "points": [{"x": 1, "y": 1, "label": 1}]},
            ]))
        logits = torch.zeros((2, 1, 2, 2))
        with self.assertRaisesRegex(ValueError, "exactly one tracked object"):
            tracking._DefaultSam2Backend._combine([1, 2], logits)

    def test_node_emits_full_length_with_uncomputed_empty(self):
        class TowardEndBackend:
            def run_multi(self, image, keyframes, direction, frame_count, obj_id=1, on_frame=None):
                te = {}
                first = min(int(k["frameIndex"]) for k in keyframes)
                for i in range(first, frame_count):
                    m = np.zeros((2, 2), dtype=np.uint8)
                    m[0, 0] = 1
                    te[i] = m
                    on_frame("towardEnd", i)
                return te, {}

        reports = []

        class Reporter:
            def __init__(self, **kwargs):
                reports.append(("init", kwargs["coverage"], kwargs["span_ms"]))

            def stage(self, stage):
                reports.append(("stage", stage))

            def frame(self, stage, idx):
                reports.append(("frame", stage, idx))

            def flush(self):
                reports.append(("flush",))

        node = tracking.BGXAnimateMultiKeyframeMask()
        node._backend_factory = lambda _: TowardEndBackend()
        node._progress_factory = Reporter
        payload = json.dumps([
            {"keyframeId": "kf0", "frameIndex": 2, "timeMs": 2, "points": [{"x": 0, "y": 0, "label": 1}]},
        ])
        batch, covered_json = node.segment(
            {}, torch.zeros((5, 2, 2, 3)), payload, "towardEnd", 5,
            request_id="req-1", selection_digest="sha256:s", frame_times_json="[0, 40, 80, 120, 160]", span_ms=200,
        )
        self.assertEqual(reports[0], ("init", [2, 3, 4], 200))
        self.assertEqual(reports[1], ("stage", "loading"))
        self.assertEqual([r[2] for r in reports if r[0] == "frame"], [2, 3, 4])
        self.assertEqual(reports[-1], ("flush",))
        with self.assertRaises(ValueError):
            node.segment({}, torch.zeros((5, 2, 2, 3)), payload, "towardEnd", 5,
                         request_id="req-1", selection_digest="sha256:s", frame_times_json="[0, 40]", span_ms=200)
        self.assertEqual(tuple(batch.shape), (5, 2, 2))
        self.assertEqual(batch[0].sum().item(), 0)
        self.assertEqual(batch[1].sum().item(), 0)
        self.assertGreater(batch[2].sum().item(), 0)
        self.assertEqual(json.loads(covered_json), [2, 3, 4])


if __name__ == "__main__":
    unittest.main()
