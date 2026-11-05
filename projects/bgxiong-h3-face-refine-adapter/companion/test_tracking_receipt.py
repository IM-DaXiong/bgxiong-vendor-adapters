import importlib.util
from pathlib import Path
import unittest
import tempfile

spec = importlib.util.spec_from_file_location("receipt", Path(__file__).parent / "custom_nodes/BGXFaceRefineContract/tracking.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class TrackingReceiptTest(unittest.TestCase):
    def setUp(self):
        self.analysis = {"analysisId":"a","revision":"1","digest":"d","coveredRange":{"startMs":0,"endMs":120},"shots":[{"shotId":"s0","range":{"startMs":0,"endMs":120},"candidates":[{"candidateId":"c0"}]}]}
        self.selection = {"analysisId":"a","analysisRevision":"1","analysisDigest":"d","selectionDigest":"selection","shots":[{"shotId":"s0","choice":{"kind":"target","candidateId":"c0"}}]}
        self.transform = {"source_frames":3,"source":[0,1,2],"detected":[True,True,True],"absent":[False,False,False]}
    def test_complete_tracking_is_ready(self):
        receipt = module.tracking_receipt(self.transform,25,self.analysis,self.selection)
        self.assertTrue(receipt["ready"]["ok"])
        self.assertEqual(receipt["selectionDigest"],"selection")
    def test_lost_and_missing_frames_are_not_ready(self):
        self.transform.update(source=[0,1],detected=[True,False],absent=[False,False])
        receipt = module.tracking_receipt(self.transform,25,self.analysis,self.selection)
        self.assertFalse(receipt["ready"]["ok"])
        self.assertEqual(receipt["lostRanges"][0]["startMs"],40)
        self.assertEqual(receipt["unresolvedRanges"][0]["startMs"],80)
    def test_explicit_absent_is_not_false_loss(self):
        self.selection["shots"][0]["choice"] = {"kind":"absent"}
        self.transform.update(source=[],detected=[],absent=[])
        self.assertTrue(module.tracking_receipt(self.transform,25,self.analysis,self.selection)["ready"]["ok"])
    def test_foreign_selection_rejected(self):
        self.selection["analysisDigest"] = "other"
        with self.assertRaises(ValueError): module.tracking_receipt(self.transform,25,self.analysis,self.selection)

class SessionPersistenceTest(unittest.TestCase):
    def test_session_survives_module_reload_and_rejects_identity_reuse(self):
        path = Path(__file__).parent / "custom_nodes/BGXFaceRefineContract/sessions.py"
        spec = importlib.util.spec_from_file_location("sessions", path)
        sessions = importlib.util.module_from_spec(spec); spec.loader.exec_module(sessions)
        value = {"analysis":{"analysisId":"a" * 32,"source":{"mediaId":"source"}},"detection":{"confidence":0.35}}
        with tempfile.TemporaryDirectory() as root:
            sessions.save_session(root,value)
            reloaded = importlib.util.module_from_spec(spec); spec.loader.exec_module(reloaded)
            self.assertEqual(reloaded.load_session(root,"a" * 32),value)
            value["detection"]["confidence"] = 0.5
            with self.assertRaises(ValueError): reloaded.save_session(root,value)
            with self.assertRaises(ValueError): reloaded.load_session(root,"../outside")

if __name__ == "__main__": unittest.main()
