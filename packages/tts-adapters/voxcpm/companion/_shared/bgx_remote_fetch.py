"""Remote artifact markers only — downloads are Rust SSOT (INV-TTS-DL-02)."""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any, Callable

ProgressFn = Callable[[str, str, dict[str, Any]], None]

log = logging.getLogger("bgx_remote_fetch")


def _load_remote_sources() -> dict[str, Any]:
    """Legacy provider registry (Python pip no longer uses PyPI; Rust owns downloads)."""
    here = Path(__file__).resolve()
    candidates = [
        here.parent / "remote_sources.json",
        here.parent.parent.parent / "_shared" / "remote_sources.json",
    ]
    for p in candidates:
        if p.is_file():
            import json

            return json.loads(p.read_text(encoding="utf-8"))
    return {"providers": {}}


class ManualInstallRequired(RuntimeError):
    """Tier C: artifacts missing; client must predownload via Rust."""

    def __init__(self, payload: dict[str, Any]):
        self.payload = payload
        super().__init__(payload.get("message", "REMOTE_ARTIFACT_MANUAL_REQUIRED"))


def format_manual_payload(entry: dict[str, Any], runtime_root: Path) -> dict[str, Any]:
    manual = entry.get("manualInstall") or {}
    target_template = manual.get("targetDirTemplate") or "{runtime_root}/{localRelativeDir}"
    local_dir = entry.get("localRelativeDir", "")
    target_dir = target_template.format(
        runtime_root=str(runtime_root),
        localRelativeDir=local_dir,
        version=runtime_root.name,
    )
    return {
        "code": "REMOTE_ARTIFACT_MANUAL_REQUIRED",
        "artifactId": entry.get("id", "unknown"),
        "attemptedProviders": [],
        "targetDir": target_dir,
        "requiredFiles": manual.get("verify") or manual.get("requiredFiles") or [],
        "guidePath": "本地运行时手动安装指南.txt",
        "guideAnchor": manual.get("guideAnchor", ""),
        "message": (
            f"Artifact {entry.get('id', 'unknown')} not ready. "
            f"Use client predownload or manual install: {target_dir}"
        ),
    }


def markers_present(runtime_root: Path, entry: dict[str, Any]) -> bool:
    store_root = _resolve_store_root(runtime_root)
    kind = entry.get("kind")
    if kind == "pip_requirements":
        wh = store_root / "wheelhouse"
        return wh.is_dir() and any(wh.iterdir())
    local_dir = entry.get("localRelativeDir")
    if not local_dir:
        return False
    base = store_root / local_dir
    markers = entry.get("requiredMarkers") or []
    if not markers:
        return base.is_dir() and any(base.iterdir())
    return all((base / m).exists() for m in markers)


def external_provisioned() -> bool:
    """RI033: BGX_EXTERNAL_PROVISIONED=1 -> probe/start only, never fetch (see passive_start.rs)."""
    import os

    return os.environ.get("BGX_EXTERNAL_PROVISIONED", "").strip() == "1"


def probe_only() -> bool:
    """RI033: BGX_PROBE_ONLY=1 -> report what is missing, install nothing, launch nothing."""
    import os

    return os.environ.get("BGX_PROBE_ONLY", "").strip() == "1"


def _norm_name(name: str) -> str:
    import re

    return re.sub(r"[-_.]+", "_", name).lower()


def pinned_requirements(requirements_path: Path) -> list[tuple[str, str]]:
    """`name==version` pins from a requirements file (markers / extras / comments ignored)."""
    import re

    out: list[tuple[str, str]] = []
    for raw in requirements_path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].split(";", 1)[0].strip()
        m = re.match(r"^([A-Za-z0-9][A-Za-z0-9._-]*)(?:\[[^\]]*\])?\s*==\s*([^\s,]+)$", line)
        if m:
            out.append((m.group(1), m.group(2)))
    return out


def wheelhouse_missing(requirements_path: Path, wheelhouse: Path) -> list[str]:
    """Pins with no matching wheel/sdist in `wheelhouse` as `name==version` (offline, no pip).

    Local version labels (`+cu128`) must match exactly when pinned.
    """
    have: set[tuple[str, str]] = set()
    if wheelhouse.is_dir():
        for f in wheelhouse.iterdir():
            n = f.name
            if n.endswith(".whl"):
                parts = n[:-4].split("-")
                if len(parts) >= 2:
                    have.add((_norm_name(parts[0]), parts[1].lower()))
            elif n.endswith((".tar.gz", ".zip")):
                stem = n[: -7 if n.endswith(".tar.gz") else -4]
                name, _, ver = stem.rpartition("-")
                if name:
                    have.add((_norm_name(name), ver.lower()))
    missing = []
    for name, ver in pinned_requirements(requirements_path):
        if (_norm_name(name), ver.lower()) not in have:
            missing.append(f"{name}=={ver}")
    return missing


def _resolve_store_root(runtime_root: Path) -> Path:
    import os

    env = os.environ.get("BGX_TTS_STORE_ROOT", "").strip()
    if env:
        return Path(env)
    return runtime_root.parent / "_store"


def ensure_pip_requirements(
    requirements_path: Path,
    venv_python: Path,
    entry: dict[str, Any],
    runtime_root: Path,
    on_progress: ProgressFn,
    pip_marker: Path,
) -> None:
    if pip_marker.is_file():
        on_progress("pip_skip", "pip dependencies already installed; skipping", {})
        return
    if (external_provisioned() or probe_only()) and requirements_path.is_file():
        wheelhouse = _resolve_store_root(runtime_root) / "wheelhouse"
        missing = wheelhouse_missing(requirements_path, wheelhouse)
        if missing:
            payload = format_manual_payload(entry, runtime_root)
            payload.update(
                {
                    "code": "WHEELHOUSE_INCOMPLETE",
                    "missingPackages": missing,
                    "targetDir": str(wheelhouse),
                    "message": "WHEELHOUSE_INCOMPLETE missing=" + ",".join(missing),
                }
            )
            on_progress("wheelhouse_incomplete", payload["message"], payload)
            raise ManualInstallRequired(payload)
    if not markers_present(runtime_root, entry):
        payload = format_manual_payload(entry, runtime_root)
        on_progress("manual_required", payload["message"], payload)
        raise ManualInstallRequired(payload)
    on_progress("pip_skip", "wheelhouse ready; PS1 installs from BGX_TTS_STORE_ROOT", {})


def ensure_huggingface_repo(
    entry: dict[str, Any],
    runtime_root: Path,
    on_progress: ProgressFn,
) -> Path:
    if markers_present(runtime_root, entry):
        store = _resolve_store_root(runtime_root)
        local = store / entry["localRelativeDir"]
        on_progress("artifact_skip", f"Artifact {entry.get('id')} already present", {})
        return local
    payload = format_manual_payload(entry, runtime_root)
    on_progress("manual_required", payload["message"], payload)
    raise ManualInstallRequired(payload)


def ensure_artifacts(manifest: dict[str, Any], runtime_root: Path, on_progress: ProgressFn) -> None:
    artifacts = manifest.get("remoteArtifacts") or []
    for entry in artifacts:
        kind = entry.get("kind")
        if kind == "huggingface_repo":
            ensure_huggingface_repo(entry, runtime_root, on_progress)
        elif kind == "pip_requirements":
            continue
        else:
            raise ValueError(f"unsupported remoteArtifacts kind: {kind}")
