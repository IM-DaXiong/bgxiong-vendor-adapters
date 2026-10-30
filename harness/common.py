# -*- coding: utf-8 -*-
"""Shared helpers for local-only A05 ops checklist."""
from __future__ import annotations
import hashlib, json, os, sys, time, urllib.request, urllib.error
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def env(name: str, default: str | None = None) -> str:
    v = os.environ.get(name, default)
    if v is None or not str(v).strip():
        raise SystemExit(f"missing env {name}")
    return str(v).strip()

def optional(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()

def base_url() -> str:
    return optional("H3_FACE_COMFY_BASE", "http://127.0.0.1:8189").rstrip("/")

def out_dir() -> Path:
    p = Path(optional("H3_FACE_OUT_DIR", str(ROOT / "verification" / "out")))
    p.mkdir(parents=True, exist_ok=True)
    return p

def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

def http_json(method: str, url: str, body=None, timeout=600):
    data = None
    headers = {"Content-Type": "application/json", "User-Agent": "h3-face-refine-harness/1.0"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
            return json.loads(raw.decode("utf-8"))
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", errors="replace")
        raise SystemExit(f"HTTP {e.code} {url}: {detail}") from e
    except Exception as e:
        raise SystemExit(f"request failed {url}: {e}") from e

def save_json(name: str, obj) -> Path:
    path = out_dir() / name
    path.write_text(json.dumps(obj, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("SAVED", path)
    return path
