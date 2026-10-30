# -*- coding: utf-8 -*-
"""A05 op1: subjects.analyze via POST /h3_facerefine/scan → Lane A analysis-receipt shape."""
from __future__ import annotations
import hashlib, json, os, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import base_url, env, http_json, save_json, sha256_file

PLUGIN = "local.thirdparty.h3-face-refine"
OP = f"{PLUGIN}/subjects.analyze"

def main():
    video = Path(env("H3_FACE_SOURCE_VIDEO"))
    if not video.is_file():
        raise SystemExit(f"source video not found: {video}")
    body = {
        "video": str(video),
        "detector": os.environ.get("H3_FACE_DETECTOR", "bbox/face_yolov8m.pt"),
        "confidence": float(os.environ.get("H3_FACE_CONFIDENCE") or "0.35"),
        "cut_detection": os.environ.get("H3_FACE_CUT") or "auto",
        "cut_threshold": 3.0,
        "select": os.environ.get("H3_FACE_SELECT_HINT") or "largest_face",
        "force": False,
    }
    print("ANALYZE", body["video"])
    raw = http_json("POST", f"{base_url()}/h3_facerefine/scan", body)
    if raw.get("error"):
        raise SystemExit(f"scan error: {raw['error']}")

    w = float(raw.get("width") or 1)
    h = float(raw.get("height") or 1)
    fps = float(raw.get("fps") or 24)
    frames = int(raw.get("frames") or 0)
    end_ms = int(round(frames / fps * 1000)) if frames else 0
    content_digest = "sha256:" + sha256_file(video)
    det_digest = "sha256:" + hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()

    shots = []
    for i, shot in enumerate(raw.get("shots") or []):
        a, b = (shot.get("segment") or [0, 0])[:2]
        start_ms = int(round(float(a) / fps * 1000))
        end_shot = int(round(float(b) / fps * 1000))
        frame_idx = int(shot.get("frame") or 0)
        rep_ms = int(round(frame_idx / fps * 1000)) if frame_idx >= 0 else start_ms
        cands = []
        for ci, bx in enumerate(shot.get("boxes") or []):
            x0, y0, x1, y1 = [float(v) for v in bx]
            # upstream boxes are normalized 0-1 → display-upright pixels
            x = x0 * w
            y = y0 * h
            width = max(0.0, (x1 - x0) * w)
            height = max(0.0, (y1 - y0) * h)
            cands.append({
                "candidateId": f"c{ci}",
                "box": {"x": x, "y": y, "width": width, "height": height},
                "previewMediaId": None,
                "score": None,
            })
        shots.append({
            "shotId": f"s{i}",
            "range": {"startMs": start_ms, "endMs": end_shot},
            "representativeFrameMs": max(0, rep_ms),
            "candidates": cands,
            "_cardHasJpg": bool(shot.get("jpg")),
        })

    digest_body = [{k: v for k, v in s.items() if not k.startswith("_")} for s in shots]
    digest = "sha256:" + hashlib.sha256(json.dumps(digest_body, sort_keys=True).encode()).hexdigest()
    analysis_id = hashlib.sha256((str(video) + digest).encode()).hexdigest()[:32]
    source = {
        "mediaId": "local-source",
        "contentDigest": content_digest,
        "selectedRange": {"startMs": 0, "endMs": end_ms},
        "displayGeometry": {
            "widthPx": int(w),
            "heightPx": int(h),
            "rotationDegrees": 0,
            "mirrored": False,
        },
        "timeBase": "ms-from-selected-range-start",
        "packageId": PLUGIN,
        "operationId": OP,
        "schemaDigest": "sha256:see-locks/schema-digests.json",
        "detectionParamsDigest": det_digest,
    }
    receipt = {
        "analysisId": analysis_id,
        "revision": "1",
        "digest": digest,
        "source": source,
        "coveredRange": {"startMs": 0, "endMs": end_ms},
        "page": {"cursor": None, "nextCursor": None, "complete": True},
        "shots": [{k: v for k, v in s.items() if not k.startswith("_")} for s in shots],
        "_localOnly": {
            "path": str(video),
            "frames": frames,
            "fps": fps,
            "vendorRawKeys": sorted(raw.keys()),
            "cardJpgPresent": [s.get("_cardHasJpg") for s in shots],
        },
    }
    save_json("analyze.receipt.json", receipt)
    save_json("analyze.summary.json", [
        {"shotId": s["shotId"], "faces": len(s["candidates"]), "range": s["range"]} for s in receipt["shots"]
    ])
    print("OK analyze shots=", len(shots), "(Lane A receipt shape)")
    print("NEXT: edit selection.json — pick distant woman candidateId, not near man")

if __name__ == "__main__":
    main()
