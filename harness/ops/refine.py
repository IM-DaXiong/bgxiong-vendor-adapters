# -*- coding: utf-8 -*-
"""A05 op3: video.refine — freeze checklist + explicit Comfy submit gate."""
from __future__ import annotations
import hashlib, json, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import base_url, env, optional, out_dir, save_json, sha256_file

def main():
    video = Path(env("H3_FACE_SOURCE_VIDEO"))
    appearance = Path(optional("H3_FACE_APPEARANCE_REF", "")) if optional("H3_FACE_APPEARANCE_REF") else None
    identity = Path(optional("H3_FACE_IDENTITY_REF", "")) if optional("H3_FACE_IDENTITY_REF") else None
    receipt = json.loads((out_dir() / "analyze.receipt.json").read_text(encoding="utf-8"))
    selection = json.loads((out_dir() / "selection.json").read_text(encoding="utf-8"))
    bindings = json.loads((out_dir() / "preview.bindings.json").read_text(encoding="utf-8"))
    prompt = optional("H3_FACE_PROMPT", "refine the selected face identity, keep lighting consistent")
    freeze = {
        "source": {
            "mediaId": "local-source",
            "contentDigest": sha256_file(video),
            "range": receipt.get("coveredRange"),
            "display": {
                "widthPx": int(receipt.get("sourceMeta", {}).get("width") or 0),
                "heightPx": int(receipt.get("sourceMeta", {}).get("height") or 0),
                "orientation": "upright",
            },
            "timeBaseMs": 1,
            "package": {
                "pluginId": "local.thirdparty.h3-face-refine",
                "pluginVersion": "1.0.0-a04",
                "operationId": "local.thirdparty.h3-face-refine/video.refine",
                "schemaDigest": "see locks/schema-digests.json",
                "detectionDigest": receipt.get("digest"),
            },
        },
        "analysis": {
            "analysisId": receipt["analysisId"],
            "revision": receipt["revision"],
            "digest": receipt["digest"],
        },
        "selection": selection,
        "tracking": {
            "selectionDigest": hashlib.sha256(json.dumps(selection, sort_keys=True).encode()).hexdigest(),
            "previewMediaId": "pending",
            "coveredRange": receipt.get("coveredRange"),
            "lostSpans": [],
            "ready": {"ok": True, "reason": None},
            "reportText": None,
        },
        "package": {
            "pluginId": "local.thirdparty.h3-face-refine",
            "pluginVersion": "1.0.0-a04",
            "upstreamNodeVersion": "1.1.2",
            "upstreamCommit": "d8521d14fe0d721d80cd9417fff5a559cbc21aba",
            "operationId": "local.thirdparty.h3-face-refine/video.refine",
            "dependencyDigest": None,
        },
        "references": [],
        "prompt": prompt,
        "params": {
            "denoise": float(optional("H3_FACE_DENOISE", "0.45")),
            "smallFaceStrength": float(optional("H3_FACE_SMALL", "0.8")),
            "largeFaceStrength": float(optional("H3_FACE_LARGE", "0.35")),
            "face_pick": bindings.get("face_pick"),
            "absent_shots": bindings.get("absent_shots"),
            "samEnabled": optional("H3_FACE_SAM", "0") == "1",
        },
    }
    if appearance and appearance.is_file():
        freeze["references"].append({"role": "appearance", "mediaId": "appearance", "order": 0, "localPath": str(appearance)})
    if identity and identity.is_file():
        freeze["references"].append({"role": "identity", "mediaId": "identity", "order": 1, "localPath": str(identity)})
    save_json("refine.freeze.json", freeze)
    print("OK freeze written. Review tracking before costly refine.")
    print("Comfy base:", base_url())
    if optional("H3_FACE_SUBMIT_REFINE") != "1":
        print("Set H3_FACE_SUBMIT_REFINE=1 after human review of preview to queue refine.")
        return
    raise SystemExit("SUBMIT_REFINE gated: assemble full API prompt with local MiniMax-H3 weight filenames first (see OPS-CHECKLIST).")

if __name__ == "__main__":
    main()
