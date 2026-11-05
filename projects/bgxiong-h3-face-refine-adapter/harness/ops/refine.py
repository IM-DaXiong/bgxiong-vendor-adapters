# -*- coding: utf-8 -*-
"""A05 op3: video.refine — Lane A freeze + selection/tracking checklist."""
from __future__ import annotations
import hashlib, json, os, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "companion"))
from common import base_url, env, optional, out_dir, save_json, sha256_file
from face_refine_ops import freeze_user_params  # noqa: E402

PLUGIN = "local.thirdparty.h3-face-refine"

def main():
    video = Path(env("H3_FACE_SOURCE_VIDEO"))
    receipt = json.loads((out_dir() / "analyze.receipt.json").read_text(encoding="utf-8"))
    selection = json.loads((out_dir() / "selection.json").read_text(encoding="utf-8"))
    bindings = json.loads((out_dir() / "preview.bindings.json").read_text(encoding="utf-8"))
    tracking_path = out_dir() / "preview.tracking.json"
    tracking = json.loads(tracking_path.read_text(encoding="utf-8")) if tracking_path.is_file() else {
        "selectionDigest": selection.get("selectionDigest"),
        "previewMediaId": "pending-preview",
        "coveredRange": receipt.get("coveredRange"),
        "lostRanges": [],
        "unresolvedRanges": [],
        "ready": {"ok": False, "reasons": ["preview not completed"]},
    }
    prompt = optional("H3_FACE_PROMPT", "refine the selected face identity, keep lighting consistent")
    appearance = optional("H3_FACE_APPEARANCE_REF")
    identity = optional("H3_FACE_IDENTITY_REF")
    reference_media_ids = []
    identity_media_ids = []
    if appearance:
        reference_media_ids.append("appearance")
    if identity:
        identity_media_ids.append("identity")
    user_params = {
        "denoise": float(optional("H3_FACE_DENOISE", "0.45")),
        "smallFaceStrength": float(optional("H3_FACE_SMALL", "0.8")),
        "largeFaceStrength": float(optional("H3_FACE_LARGE", "0.35")),
        "samEnabled": optional("H3_FACE_SAM", "0") == "1",
        "scaleMode": optional("H3_FACE_SCALE_MODE", "absolute_px"),
        "face_pick": bindings.get("face_pick"),
        "absent_shots": bindings.get("absent_shots"),
        "identityMediaIds": identity_media_ids,
    }
    # Validate unique-field reader (no params/references aliases).
    _ = freeze_user_params({"userParams": user_params})

    freeze = {
        "source": receipt.get("source"),
        "analysis": {
            "analysisId": receipt["analysisId"],
            "revision": str(receipt["revision"]),
            "digest": receipt["digest"],
        },
        "selectionDigest": selection.get("selectionDigest"),
        "trackingDigest": tracking.get("digest") or tracking.get("selectionDigest"),
        "packageVersion": "1.0.0-a04",
        "dependencyVersions": {
            "upstream-node": "1.1.2",
            "upstreamCommit": "d8521d14fe0d721d80cd9417fff5a559cbc21aba",
        },
        "operationId": f"{PLUGIN}/video.refine",
        "schemaDigest": "pending-host",
        "referenceMediaIds": reference_media_ids,
        "prompt": prompt,
        "userParams": user_params,
        # Local harness-only (not part of public freeze schema; stripped before package submit).
        "_localOnly": {
            "sourcePath": str(video),
            "contentDigest": "sha256:" + sha256_file(video),
            "comfyBase": base_url(),
            "selection": selection,
            "tracking": tracking,
            "appearancePath": appearance,
            "identityPath": identity,
        },
    }
    save_json("refine.freeze.json", freeze)
    print("OK freeze (Lane A). Review tracking before costly refine.")
    if optional("H3_FACE_SUBMIT_REFINE") != "1":
        print("Set H3_FACE_SUBMIT_REFINE=1 after preview OK to queue refine.")
        return
    raise SystemExit("SUBMIT_REFINE gated — assemble full API prompt with local MiniMax-H3 weight filenames first.")

if __name__ == "__main__":
    main()
