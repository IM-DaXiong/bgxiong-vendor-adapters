# -*- coding: utf-8 -*-
"""A05 op2: subjects.preview — bind selection → face_pick / absent_shots; submit preview graph if Comfy ready."""
from __future__ import annotations
import json, sys, uuid
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import ROOT, base_url, env, http_json, optional, out_dir, save_json

def selection_to_bindings(selection: dict):
    picks, absent = [], []
    for s in selection.get("shots") or []:
        sid = s["shotId"]
        kind = (s.get("decision") or {}).get("kind")
        if kind == "absent":
            absent.append(sid)
        elif kind == "target":
            cid = (s.get("decision") or {}).get("candidateId") or ""
            idx = cid[1:] if cid.startswith("c") else cid
            picks.append(f"{sid}:{idx}")
        else:
            raise SystemExit(f"incomplete decision for {sid}: unset is not absent")
    return ",".join(picks), ",".join(absent)

def main():
    sel_path = Path(optional("H3_FACE_SELECTION_JSON", str(out_dir() / "selection.json")))
    if not sel_path.is_file():
        # auto-create template from analyze receipt if present
        receipt_path = out_dir() / "analyze.receipt.json"
        if receipt_path.is_file():
            receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
            template = {
                "analysisId": receipt["analysisId"],
                "analysisRevision": receipt["revision"],
                "analysisDigest": receipt["digest"],
                "shots": [
                    {"shotId": s["shotId"], "decision": {"kind": "target", "candidateId": "c0"}}
                    for s in receipt.get("shots") or []
                ],
                "NOTE": "EDIT candidateId to the distant woman (often not c0/largest). Then re-run preview.",
            }
            sel_path.write_text(json.dumps(template, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            print("WROTE template selection — edit before trusting preview:", sel_path)
            raise SystemExit(3)
        raise SystemExit(f"missing selection json: {sel_path}")
    selection = json.loads(sel_path.read_text(encoding="utf-8"))
    face_pick, absent_shots = selection_to_bindings(selection)
    bindings = {
        "face_pick": face_pick,
        "absent_shots": absent_shots,
        "selection": selection,
        "workflow": "workflows/preview.track.api.json",
        "comfyBase": base_url(),
    }
    save_json("preview.bindings.json", bindings)
    # Soft-check object_info for required classes; do not auto-queue a long render without explicit flag
    import os
    try:
        info = http_json("GET", f"{base_url()}/object_info")
        required = ["H3FaceSelect", "H3FaceTrackCrop"]
        missing = [c for c in required if c not in info]
        bindings["probe"] = {"missingClasses": missing, "ok": not missing}
        save_json("preview.bindings.json", bindings)
        if missing:
            print("BLOCKER preview: missing classes", missing)
            print("Prepare isolated runtime (companion/scripts/prepare-runtime.ps1) and restart Comfy with extra_model_paths.")
            raise SystemExit(2)
    except SystemExit:
        raise
    except Exception as e:
        print("WARN cannot reach Comfy object_info:", e)
        bindings["probe"] = {"ok": False, "error": str(e)}
        save_json("preview.bindings.json", bindings)
        raise SystemExit(2)
    if os.environ.get("H3_FACE_SUBMIT_PREVIEW") != "1":
        print("OK bindings ready. Set H3_FACE_SUBMIT_PREVIEW=1 to queue Comfy preview workflow.")
        print("face_pick=", face_pick, "absent_shots=", absent_shots)
        return
    # Submit is intentionally explicit: full API graph assembly depends on local weight names.
    raise SystemExit("H3_FACE_SUBMIT_PREVIEW=1 set but full prompt assembly still requires managed weight map — see harness/OPS-CHECKLIST.md")

if __name__ == "__main__":
    main()
