# -*- coding: utf-8 -*-
"""A05 op2: subjects.preview — Lane A selection.choice → face_pick / absent_shots."""
from __future__ import annotations
import hashlib, json, os, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import base_url, http_json, optional, out_dir, save_json

def selection_to_bindings(selection: dict):
    picks, absent = [], []
    for s in selection.get("shots") or []:
        sid = s["shotId"]
        choice = s.get("choice") or s.get("decision")  # decision = legacy alias
        if not choice:
            raise SystemExit(f"incomplete choice for {sid}: unset is not absent")
        kind = choice.get("kind")
        if kind == "absent":
            absent.append(sid)
        elif kind == "target":
            cid = choice.get("candidateId") or ""
            idx = cid[1:] if cid.startswith("c") and cid[1:].isdigit() else cid
            # upstream face_pick uses per-shot index; shot id sN → N for compact form
            shot_idx = sid[1:] if sid.startswith("s") and sid[1:].isdigit() else sid
            picks.append(f"{shot_idx}:{idx}")
        else:
            raise SystemExit(f"invalid choice kind for {sid}: {kind}")
    return ",".join(picks), ",".join(absent)

def main():
    sel_path = Path(optional("H3_FACE_SELECTION_JSON", str(out_dir() / "selection.json")))
    receipt_path = out_dir() / "analyze.receipt.json"
    if not sel_path.is_file():
        if not receipt_path.is_file():
            raise SystemExit("run analyze.py first")
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
        shots = []
        for s in receipt.get("shots") or []:
            # default c0 — MUST edit for 107-frame woman case
            shots.append({"shotId": s["shotId"], "choice": {"kind": "target", "candidateId": "c0"}})
        body = {
            "analysisId": receipt["analysisId"],
            "analysisRevision": str(receipt["revision"]),
            "analysisDigest": receipt["digest"],
            "shots": shots,
            "selectionDigest": "pending",
            "NOTE": "EDIT candidateId to distant woman (often not c0/largest). Then re-run preview.",
        }
        body["selectionDigest"] = "sha256:" + hashlib.sha256(
            json.dumps({k: body[k] for k in ("analysisId", "analysisRevision", "analysisDigest", "shots")}, sort_keys=True).encode()
        ).hexdigest()
        sel_path.write_text(json.dumps(body, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print("WROTE template selection (Lane A choice shape) — edit before trusting:", sel_path)
        raise SystemExit(3)

    selection = json.loads(sel_path.read_text(encoding="utf-8"))
    # normalize decision→choice
    for s in selection.get("shots") or []:
        if "decision" in s and "choice" not in s:
            s["choice"] = s.pop("decision")
    if "selectionDigest" not in selection:
        selection["selectionDigest"] = "sha256:" + hashlib.sha256(
            json.dumps({k: selection.get(k) for k in ("analysisId", "analysisRevision", "analysisDigest", "shots")}, sort_keys=True).encode()
        ).hexdigest()
        sel_path.write_text(json.dumps(selection, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    face_pick, absent_shots = selection_to_bindings(selection)
    bindings = {
        "face_pick": face_pick,
        "absent_shots": absent_shots,
        "selection": selection,
        "workflow": "workflows/preview.track.api.json",
        "comfyBase": base_url(),
    }
    try:
        info = http_json("GET", f"{base_url()}/object_info")
        required = ["H3FaceSelect", "H3FaceTrackCrop"]
        missing = [c for c in required if c not in info]
        bindings["probe"] = {"missingClasses": missing, "ok": not missing}
        save_json("preview.bindings.json", bindings)
        if missing:
            print("BLOCKER preview: missing classes", missing)
            raise SystemExit(2)
    except SystemExit:
        raise
    except Exception as e:
        bindings["probe"] = {"ok": False, "error": str(e)}
        save_json("preview.bindings.json", bindings)
        print("BLOCKER cannot reach Comfy:", e)
        raise SystemExit(2)

    # Lane A tracking-receipt skeleton (preview media filled after submit)
    tracking = {
        "selectionDigest": selection["selectionDigest"],
        "previewMediaId": "pending-preview",
        "coveredRange": {"startMs": 0, "endMs": 0},
        "lostRanges": [],
        "unresolvedRanges": [],
        "ready": {"ok": False, "reasons": ["awaiting Comfy preview render"]},
    }
    if receipt_path.is_file():
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
        tracking["coveredRange"] = receipt.get("coveredRange") or tracking["coveredRange"]
    save_json("preview.tracking.json", tracking)
    save_json("preview.bindings.json", bindings)

    if os.environ.get("H3_FACE_SUBMIT_PREVIEW") != "1":
        print("OK bindings ready (Lane A). face_pick=", face_pick, "absent=", absent_shots)
        print("Set H3_FACE_SUBMIT_PREVIEW=1 when weight map ready to queue preview.")
        return
    raise SystemExit("H3_FACE_SUBMIT_PREVIEW=1 set but full prompt assembly still gated — see OPS-CHECKLIST.md")

if __name__ == "__main__":
    main()
