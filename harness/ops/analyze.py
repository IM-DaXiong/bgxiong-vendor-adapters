# -*- coding: utf-8 -*-
"""A05 op1: subjects.analyze via upstream POST /h3_facerefine/scan (no browser)."""
from __future__ import annotations
import json, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import base_url, env, http_json, save_json, sha256_file

def main():
    video = Path(env("H3_FACE_SOURCE_VIDEO"))
    if not video.is_file():
        raise SystemExit(f"source video not found: {video}")
    body = {
        "video": str(video),
        "detector": env("H3_FACE_DETECTOR") if False else "bbox/face_yolov8m.pt",
        "confidence": float((__import__("os").environ.get("H3_FACE_CONFIDENCE") or "0.35")),
        "cut_detection": (__import__("os").environ.get("H3_FACE_CUT") or "auto"),
        "cut_threshold": 3.0,
        "select": (__import__("os").environ.get("H3_FACE_SELECT_HINT") or "largest_face"),
        "force": False,
    }
    # allow detector override
    import os
    if os.environ.get("H3_FACE_DETECTOR"):
        body["detector"] = os.environ["H3_FACE_DETECTOR"]
    print("ANALYZE", body["video"])
    raw = http_json("POST", f"{base_url()}/h3_facerefine/scan", body)
    if raw.get("error"):
        raise SystemExit(f"scan error: {raw['error']}")
    # Map to package-local analysis receipt (same rules as adapter)
    w = float(raw.get("width") or 1)
    h = float(raw.get("height") or 1)
    fps = float(raw.get("fps") or 24)
    shots = []
    for i, shot in enumerate(raw.get("shots") or []):
        a, b = (shot.get("segment") or [0, 0])[:2]
        cands = []
        for ci, bx in enumerate(shot.get("boxes") or []):
            x0, y0, x1, y1 = bx
            cands.append({
                "candidateId": f"c{ci}",
                "box": {"x0": x0 * w, "y0": y0 * h, "x1": x1 * w, "y1": y1 * h},
                "score": None,
                "thumbnailMediaId": None,
            })
        shots.append({
            "shotId": f"s{i}",
            "startMs": int(round(float(a) / fps * 1000)),
            "endMs": int(round(float(b) / fps * 1000)),
            "representativeFrameIndex": shot.get("frame", -1),
            "previewMediaId": None,
            "candidates": cands,
            "cardHasJpg": bool(shot.get("jpg")),
        })
    digest_src = json.dumps(shots, ensure_ascii=False, sort_keys=True)
    digest = __import__("hashlib").sha256(digest_src.encode()).hexdigest()
    analysis_id = __import__("hashlib").sha256((str(video) + digest).encode()).hexdigest()[:32]
    receipt = {
        "analysisId": analysis_id,
        "revision": 1,
        "digest": digest,
        "coveredRange": {"startMs": 0, "endMs": int(round(float(raw.get("frames") or 0) / fps * 1000))},
        "page": {"cursor": None, "hasMore": False, "nextCursor": None},
        "shots": shots,
        "sourceMeta": {
            "pathLocalOnly": str(video),
            "contentDigest": sha256_file(video),
            "frames": raw.get("frames"),
            "width": w,
            "height": h,
            "fps": fps,
        },
        "vendorRawKeys": sorted(raw.keys()),
    }
    save_json("analyze.receipt.json", receipt)
    # also dump per-shot candidate summary without jpg base64
    summary = [{"shotId": s["shotId"], "faces": len(s["candidates"]), "startMs": s["startMs"], "endMs": s["endMs"]} for s in shots]
    save_json("analyze.summary.json", summary)
    print("OK analyze shots=", len(shots))
    print("NEXT: write selection JSON (target distant woman) then run ops/preview.py")

if __name__ == "__main__":
    main()
