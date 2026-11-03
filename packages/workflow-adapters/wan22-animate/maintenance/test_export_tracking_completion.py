"""CPU-only mapper behavior fixtures; these are not remote completion evidence."""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from PIL import Image
from export_tracking_completion import export


class TrackingCompletionExportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "preserved"
        self.source.mkdir()
        self.task = "12345678-1234-4234-8234-123456789abc"
        self.prompt = "remote-original"
        self.make_fixture()

    def put(self, name, value):
        path = self.source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value), encoding="utf-8")

    def seal(self):
        self.put("preserved-inventory.json", [{"path": str(p), "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in self.source.rglob("*") if p.is_file() and p.name != "preserved-inventory.json"])

    def make_fixture(self):
        (self.source / "source.submitted.mp4").write_bytes(b"unit-source-bytes")
        source_sha = hashlib.sha256(b"unit-source-bytes").hexdigest()
        inp = {"sourceIdentity": {"contentDigest": "sha256:" + source_sha}, "selection": {"selectionDigest": "ab" * 32}, "range": {"startMs": 0, "endMs": 1000}, "direction": "towardEnd"}
        request = {"input": inp, "configuration": {"endpointBaseUrl": "http://fixture.invalid"}, "inputSchemaDigest": "cd" * 32, "operationId": "local.test/tracking"}
        resolved = [{"keyframeId": "kf", "frameIndex": 0, "resolvedTimeMs": 0}]
        exp = {"request_id": self.task, "selection_digest": inp["selection"]["selectionDigest"], "source_digest": inp["sourceIdentity"]["contentDigest"], "direction": "towardEnd", "input_schema_digest": request["inputSchemaDigest"], "operation_id": request["operationId"], "operation_version": "1.0.0", "range_start_ms": 0, "range_end_ms": 1000, "resolved_keyframes_json": json.dumps(resolved)}
        graph = {"track.export": {"inputs": exp}, "sam.multi": {"inputs": {"frame_times_json": "[0]"}}}
        self.put("checkpoint.original.json", {"requestId": self.task, "state": "acceptance_unknown", "mediaIdentities": [{"id": "source", "digest": source_sha}]})
        self.put("frozen.original.json", {"request": request})
        self.put("sdk-submit/execution.invocation.json", {"operation": "submit", "requestId": self.task, "payloadJson": json.dumps(request)})
        post = {"method": "POST", "url": "http://fixture.invalid/prompt", "requestId": self.task, "body": {"client_id": self.task, "prompt": graph}}
        (self.source / "sdk-submit/before-send.jsonl").write_text(json.dumps(post) + "\n", encoding="utf-8")
        self.put("history.http.json", {self.prompt: {"prompt": [1, self.prompt, graph, {"client_id": self.task}], "status": {"status_str": "success", "completed": True, "messages": [["execution_success", {"prompt_id": self.prompt}]]}}})
        png = self.source / "remote-artifact/masks/frame_00000.png"
        png.parent.mkdir(parents=True)
        Image.new("L", (2, 2), 255).save(png)
        artifact_digest = hashlib.sha256(b"0:" + bytes([255] * 4)).hexdigest()
        self.manifest = {"requestId": self.task, "selectionDigest": inp["selection"]["selectionDigest"], "sourceDigest": inp["sourceIdentity"]["contentDigest"], "direction": "towardEnd", "range": inp["range"], "operationRef": {"inputSchemaDigest": request["inputSchemaDigest"], "operationId": request["operationId"], "operationVersion": "1.0.0"}, "resolvedKeyframes": resolved, "ready": True, "readyReason": None, "coveredRanges": [inp["range"]], "uncomputedRanges": [], "lostRanges": [], "skipRanges": [], "outputGeometry": {"width": 2, "height": 2}, "outputFps": None, "frameCount": 1, "frames": [{"frameIndex": 0, "timeMs": 0, "state": "covered", "relPath": "masks/frame_00000.png"}], "digest": "sha256:" + artifact_digest}
        self.put("manifest.http.json", self.manifest)
        self.put("remote-artifact/manifest.json", self.manifest)
        self.seal()

    def run_export(self, name="exported"):
        with contextlib.redirect_stdout(io.StringIO()):
            return export(self.source, self.root / name)

    def test_exports_bound_completed_data_and_real_files_without_handles(self):
        completion = self.run_export()
        data = json.loads(completion.read_text())
        self.assertEqual(set(data), {"result", "media"})
        self.assertEqual(data["result"]["kind"], "completed")
        self.assertEqual(data["result"]["vendorTaskId"], self.prompt)
        self.assertEqual(data["result"]["outputs"], [])
        self.assertNotIn("resumeToken", data["result"])
        self.assertEqual([m["role"] for m in data["media"]], ["maskFrame", "framePage", "frameIndex"])
        for m in data["media"]:
            self.assertNotIn("handle", m)
            self.assertEqual(m["sha256"], hashlib.sha256(Path(m["path"]).read_bytes()).hexdigest())
        page = json.loads(Path(data["media"][1]["path"]).read_text())
        self.assertEqual(page["frames"][0]["maskPageRef"]["artifactId"], data["media"][0]["id"])
        index = json.loads(Path(data["media"][2]["path"]).read_text())
        self.assertEqual(index["pages"][0]["mediaId"], data["media"][1]["id"])
        self.assertEqual(data["result"]["data"]["framePageIndexRef"], data["media"][2]["id"])

    def test_changed_preserved_png_is_rejected_before_any_export(self):
        Image.new("L", (2, 2), 0).save(self.source / "remote-artifact/masks/frame_00000.png")
        with self.assertRaisesRegex(ValueError, "evidence changed"):
            self.run_export()
        self.assertFalse((self.root / "exported").exists())

    def test_wrong_remote_client_and_wrong_sealed_pixel_digest_are_rejected(self):
        history = json.loads((self.source / "history.http.json").read_text())
        history[self.prompt]["prompt"][3]["client_id"] = "other-task"
        self.put("history.http.json", history); self.seal()
        with self.assertRaisesRegex(ValueError, "identity differs"):
            self.run_export()
        self.make_fixture_after_reset()
        self.manifest["digest"] = "sha256:" + "00" * 32
        self.put("manifest.http.json", self.manifest); self.put("remote-artifact/manifest.json", self.manifest); self.seal()
        with self.assertRaisesRegex(ValueError, "sealed artifact digest"):
            self.run_export()

    def make_fixture_after_reset(self):
        import shutil
        shutil.rmtree(self.source)
        self.source.mkdir()
        self.make_fixture()

    def test_existing_export_is_never_overwritten(self):
        completion = self.run_export()
        original = completion.read_bytes()
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.run_export()
        self.assertEqual(completion.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
