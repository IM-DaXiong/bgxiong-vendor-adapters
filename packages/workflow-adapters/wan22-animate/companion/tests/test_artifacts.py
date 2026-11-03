
from __future__ import annotations

import importlib.util
import json
import shutil
import tempfile
import unittest
from pathlib import Path

import numpy as np


def _load_artifacts():
    path = (
        Path(__file__).resolve().parents[1]
        / "custom_nodes"
        / "BGXAnimateTracking"
        / "tracking_artifacts.py"
    )
    spec = importlib.util.spec_from_file_location("bgx_animate_tracking_artifacts", path)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    # Register before exec so dataclasses / annotations resolve if added later.
    import sys
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


OP = {"operationId": "op", "operationVersion": "1.0.0", "inputSchemaDigest": "e" * 64}


class MaskArtifactTests(unittest.TestCase):
    def setUp(self):
        self.mod = _load_artifacts()
        self.tmp = Path(tempfile.mkdtemp(prefix="bgx-mask-art-"))

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def export(self, request_id, masks, covered, direction="both", times=None, span=None, fps=0.0):
        times = times if times is not None else [i * 40 for i in range(len(masks))]
        span = span if span is not None else times[-1] + 40
        return self.mod.export_masks(
            self.tmp,
            request_id=request_id,
            selection_digest="sha256:" + "a" * 64,
            direction=direction,
            width=masks[0].shape[1],
            height=masks[0].shape[0],
            fps=fps,
            masks=masks,
            covered_indices=covered,
            resolved_keyframes=[],
            source_digest="sha256:" + "b" * 64,
            range_ms={"startMs": 0, "endMs": span},
            operation_ref=OP,
            frame_times=times,
        )

    def test_export_load_roundtrip_and_digest(self):
        masks = [np.zeros((4, 6), dtype=np.float32) for _ in range(3)]
        for m in masks:
            m[1:3, 2:4] = 1.0
        result = self.export("req-abc", masks, [0, 1, 2], fps=24.0)
        self.assertEqual(result["artifactId"], "req-abc")
        self.assertTrue(result["digest"].startswith("sha256:"))
        self.assertTrue(result["manifest"]["ready"])
        self.assertEqual(result["manifest"]["outputFps"], 24.0)
        loaded, man = self.mod.load_masks(self.tmp, result["artifactId"], result["digest"])
        self.assertEqual(len(loaded), 3)
        self.assertEqual(man["digest"], result["digest"])
        np.testing.assert_array_equal((loaded[1] > 0.5), (masks[1] > 0.5))

    def test_digest_mismatch_rejected(self):
        result = self.export("req-1", [np.ones((2, 2), dtype=np.float32)], [0], direction="towardEnd")
        with self.assertRaises(ValueError):
            self.mod.load_masks(self.tmp, result["artifactId"], "sha256:" + "0" * 64)

    def test_actual_frame_times_lost_and_uncomputed_states(self):
        masks = [np.ones((2, 2), dtype=np.float32), np.zeros((2, 2), dtype=np.float32),
                 np.ones((2, 2), dtype=np.float32), np.zeros((2, 2), dtype=np.float32)]
        result = self.export("vfr-1", masks, [0, 1, 2], direction="towardEnd", times=[0, 33, 70, 101], span=130)
        man = result["manifest"]
        self.assertEqual([f["timeMs"] for f in man["frames"]], [0, 33, 70, 101])
        self.assertEqual([f["state"] for f in man["frames"]], ["covered", "lost", "covered", "uncomputed"])
        self.assertEqual(man["coveredRanges"], [{"startMs": 0, "endMs": 33}, {"startMs": 70, "endMs": 101}])
        self.assertEqual(man["lostRanges"], [{"startMs": 33, "endMs": 70}])
        self.assertEqual(man["uncomputedRanges"], [{"startMs": 101, "endMs": 130}])
        self.assertIsNone(man["outputFps"])
        self.assertFalse(man["ready"])
        with self.assertRaises(ValueError):
            self.export("bad-times", masks, [0], times=[0, 40, 40, 80], span=120)

    def test_finished_artifacts_are_never_deleted_by_age(self):
        results = [self.export(f"done-{i}", [np.ones((2, 2), dtype=np.float32)], [0]) for i in range(3)]
        for result in results:
            frames, _ = self.mod.load_masks(self.tmp, result["artifactId"], result["digest"])
            self.assertEqual(len(frames), 1)
        self.assertFalse(hasattr(self.mod, "gc_completed"))

    def test_progress_counts_unique_covered_frames_and_flushes(self):
        now = [0.0]
        root = self.mod.artifact_root(self.tmp, "prog-1")
        reporter = self.mod.ProgressReporter(
            root, request_id="prog-1", selection_digest="sha256:s", direction="both",
            frame_times=[0, 40, 80, 120, 160], coverage=[0, 1, 2, 3, 4], span_ms=200,
            clock=lambda: now[0], min_interval=0.5,
        )
        reporter.stage("loading")
        self.assertIsNotNone(reporter.frame("towardEnd", 2))
        self.assertIsNone(reporter.frame("towardEnd", 3))
        reporter.frame("towardEnd", 4)
        written = reporter.frame("towardStart", 2)
        self.assertEqual((written["stage"], written["processedFrames"], written["totalFrames"]), ("towardStart", 3, 5))
        reporter.frame("towardStart", 1)
        reporter.frame("towardStart", 0)
        final = reporter.flush()
        self.assertEqual((final["processedFrames"], final["totalFrames"]), (5, 5))
        self.assertEqual(final["coveredRanges"], [{"startMs": 0, "endMs": 200}])
        self.assertEqual(final["lastCompletedTimeMs"], 0)
        self.assertGreater(int(final["revision"]), int(written["revision"]))
        self.assertEqual(json.loads((root / "progress.json").read_text(encoding="utf-8")), final)


if __name__ == "__main__":
    unittest.main()
