# -*- coding: utf-8 -*-
"""Shared face-refine ops helpers (used by harness; package WASM mirrors the same rules in Rust).

Harness may import this for CLI development. User runtime path is the WASM adapter via Host HTTP,
not these scripts.
"""
from __future__ import annotations
from typing import Any, Dict, List, Tuple


def selection_to_bindings(selection: dict) -> Tuple[str, str]:
    """Map Lane A selection.shots[].choice → upstream face_pick / absent_shots strings."""
    picks: List[str] = []
    absent: List[str] = []
    for s in selection.get("shots") or []:
        sid = s.get("shotId") or ""
        choice = s.get("choice") or s.get("decision")
        if not choice:
            raise ValueError(f"incomplete choice for {sid}: unset is not absent")
        kind = choice.get("kind")
        if kind == "absent":
            absent.append(sid)
        elif kind == "target":
            cid = choice.get("candidateId") or ""
            idx = cid[1:] if cid.startswith("c") and cid[1:].isdigit() else cid
            shot_idx = sid[1:] if sid.startswith("s") and sid[1:].isdigit() else sid
            picks.append(f"{shot_idx}:{idx}")
        else:
            raise ValueError(f"invalid choice kind for {sid}: {kind}")
    return ",".join(picks), ",".join(absent)


def freeze_user_params(freeze: dict) -> Dict[str, Any]:
    """Read package knobs from unique freeze.userParams (no params/references aliases)."""
    params = freeze.get("userParams") or {}
    if not isinstance(params, dict):
        params = {}
    return {
        "denoise": float(params.get("denoise", 0.45)),
        "smallFaceStrength": float(params.get("smallFaceStrength", 0.8)),
        "largeFaceStrength": float(params.get("largeFaceStrength", 0.35)),
        "samEnabled": bool(params.get("samEnabled", False)),
        "scaleMode": params.get("scaleMode") or "absolute_px",
        "identityMediaIds": list(params.get("identityMediaIds") or []),
    }


def build_tracking_skeleton(selection: dict, covered_range: dict | None = None) -> dict:
    return {
        "selectionDigest": selection.get("selectionDigest"),
        "previewMediaId": None,
        "coveredRange": covered_range or {"startMs": 0, "endMs": 0},
        "lostRanges": [],
        "unresolvedRanges": [],
        "ready": {"ok": False, "reasons": ["awaiting Comfy preview render"]},
    }
