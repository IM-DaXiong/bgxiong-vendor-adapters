"""Shared sidecar bootstrap helpers (JSONL progress · SSOT for all TTS sidecars)."""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def append_bootstrap(jsonl_path: Path, phase: str, message: str, **extra: Any) -> None:
    entry: dict[str, Any] = {
        "ts": utc_now(),
        "phase": phase,
        "message": message,
    }
    entry.update(extra)
    with jsonl_path.open("a", encoding="utf-8") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def make_remote_progress_fn(jsonl_path: Path) -> Callable[[str, str, dict[str, Any]], None]:
    """Adapt bgx_remote_fetch ProgressFn (3-arg) -> append_bootstrap (2-arg + **extra)."""

    def progress(phase: str, message: str, extra: dict[str, Any]) -> None:
        append_bootstrap(jsonl_path, phase, message, **(extra or {}))

    return progress


def resolve_python(explicit: list[str] | None = None) -> str:
    import os
    import shutil

    managed = os.environ.get("BGXIONG_MANAGED_PYTHON", "").strip()
    passive = os.environ.get("BGX_EXTERNAL_PROVISIONED") == "1" or os.environ.get("BGX_PROBE_ONLY") == "1"
    if passive:
        if managed and Path(managed).is_file():
            return managed
        # RI033: external provisioning never falls back to an unmanaged PATH interpreter.
        raise RuntimeError("MANAGED_PYTHON_MISSING: BGXIONG_MANAGED_PYTHON is not set to an existing file")

    for candidate in explicit or ("python", "python3", "py"):
        found = shutil.which(candidate)
        if found:
            return found
    raise RuntimeError(
        "Python 3.10+ required on PATH. Install from https://www.python.org/downloads/"
    )
