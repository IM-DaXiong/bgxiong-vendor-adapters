import json
import os
import tempfile
from pathlib import Path


def session_path(root, analysis_id):
    if not isinstance(analysis_id, str) or len(analysis_id) != 32 or any(c not in "0123456789abcdef" for c in analysis_id):
        raise ValueError("Invalid analysis session identity")
    return Path(root) / (analysis_id + ".json")


def save_session(root, value):
    analysis = value["analysis"]
    path = session_path(root, analysis["analysisId"])
    if not isinstance(analysis.get("source"), dict) or not isinstance(value.get("detection"), dict):
        raise ValueError("Session source/detection missing")
    data = json.dumps(value, ensure_ascii=False, sort_keys=True).encode("utf-8")
    if len(data) > 4 * 1024 * 1024:
        raise ValueError("Analysis session exceeds 4 MiB")
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        if json.loads(path.read_text(encoding="utf-8")) != value:
            raise ValueError("Analysis identity already belongs to a different result")
        return
    fd, temporary = tempfile.mkstemp(prefix="analysis-", suffix=".json", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary): os.unlink(temporary)


def load_session(root, analysis_id):
    return json.loads(session_path(root, analysis_id).read_text(encoding="utf-8"))
