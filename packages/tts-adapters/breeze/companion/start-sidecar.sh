#!/usr/bin/env bash

# BreezeTTS2 TTS sidecar launcher (Unix)

set -euo pipefail



RUNTIME_ROOT="$(cd "$(dirname "$0")" && pwd)"

VENV_DIR="$RUNTIME_ROOT/.venv"

BOOTSTRAP_JSONL="$RUNTIME_ROOT/sidecar.bootstrap.jsonl"

STDERR_LOG="$RUNTIME_ROOT/sidecar.stderr.log"

PID_FILE="$RUNTIME_ROOT/sidecar.pid"

REQUIREMENTS="$RUNTIME_ROOT/requirements.txt"

SERVER_PY="$RUNTIME_ROOT/sidecar_server.py"

if [[ -f "$RUNTIME_ROOT/_shared/bgx_sidecar_common.py" ]]; then
  SHARED_DIR="$RUNTIME_ROOT/_shared"
elif [[ -f "$RUNTIME_ROOT/../_shared/bgx_sidecar_common.py" ]]; then
  SHARED_DIR="$(cd "$RUNTIME_ROOT/../_shared" && pwd)"
elif [[ -d "$RUNTIME_ROOT/../../_shared" ]]; then
  SHARED_DIR="$(cd "$RUNTIME_ROOT/../../_shared" && pwd)"
else
  SHARED_DIR="$(cd "$RUNTIME_ROOT/../_shared" && pwd)"
fi
MANIFEST="$RUNTIME_ROOT/manifest.json"



write_bootstrap() {

  local phase="$1"

  local message="$2"

  local ts

  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

  printf '{"ts":"%s","phase":"%s","message":"%s"}\n' "$ts" "$phase" "$message" >> "$BOOTSTRAP_JSONL"

}

# RI033 passive start (src-tauri/src/runtime_intake/passive_start.rs):
#   BGX_EXTERNAL_PROVISIONED=1 -> never touch the network; missing wheels -> WHEELHOUSE_INCOMPLETE missing=...
#   BGX_PROBE_ONLY=1           -> exit 3 (PROBE_ONLY_NEEDS_INSTALL) before install, exit 0 (PROBE_ONLY_READY) before launch.
BGX_EXTERNAL_PROVISIONED="${BGX_EXTERNAL_PROVISIONED:-}"
BGX_PROBE_ONLY="${BGX_PROBE_ONLY:-}"
if [[ "$BGX_EXTERNAL_PROVISIONED" == "1" || "$BGX_PROBE_ONLY" == "1" ]]; then
  export HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 PIP_NO_INDEX=1
fi
bgx_emit() { # phase message
  write_bootstrap "$1" "$2"
}
bgx_probe_needs_install() {
  if [[ "$BGX_PROBE_ONLY" == "1" ]]; then bgx_emit probe_needs_install "PROBE_ONLY_NEEDS_INSTALL $1"; exit 3; fi
}
bgx_probe_ready() {
  if [[ "$BGX_PROBE_ONLY" == "1" ]]; then bgx_emit probe_ready "PROBE_ONLY_READY"; exit 0; fi
}
bgx_pip_offline() { # pip install from local wheelhouse; on failure report missing wheels
  local out rc=0
  out="$("$@" 2>&1)" || rc=$?
  printf '%s\n' "$out" >&2
  if [[ $rc -ne 0 ]]; then
    local missing
    missing="$(printf '%s\n' "$out" | sed -nE 's/.*(No matching distribution found for|Could not find a version that satisfies the requirement)[[:space:]]+([^[:space:](]+).*/\2/p' | sort -u | paste -sd, -)"
    [[ -n "$missing" ]] && bgx_emit wheelhouse_incomplete "WHEELHOUSE_INCOMPLETE missing=$missing"
  fi
  return $rc
}



PORT=0

while [[ $# -gt 0 ]]; do

  case "$1" in

    --port)

      PORT="$2"

      shift 2

      ;;

    *)

      echo "Unknown arg: $1" >&2

      exit 1

      ;;

  esac

done



if [[ "$PORT" -le 0 ]]; then

  echo "Missing --port" >&2

  exit 1

fi



export BGX_TTS_SIDECAR_PORT="$PORT"

export PYTHONPATH="$SHARED_DIR:$RUNTIME_ROOT"

rm -f "$BOOTSTRAP_JSONL"

write_bootstrap init "Starting Breeze sidecar bootstrap (port=$PORT)"



PYTHON_CMD=""

if [[ -n "${BGXIONG_MANAGED_PYTHON:-}" && -x "${BGXIONG_MANAGED_PYTHON}" ]]; then

  ver="$("${BGXIONG_MANAGED_PYTHON}" --version 2>&1 || true)"

  if [[ "$ver" == Python\ 3.12.* ]]; then

    PYTHON_CMD="${BGXIONG_MANAGED_PYTHON}"

  fi

fi

if [[ -z "$PYTHON_CMD" ]]; then

  write_bootstrap error "BGXIONG_MANAGED_PYTHON required (Python 3.12). Download managed Python runtime in Settings."

  exit 1

fi



write_bootstrap python_check "Using Python: $PYTHON_CMD"



if [[ ! -d "$VENV_DIR" ]]; then

  write_bootstrap venv_create "Creating virtual environment .venv..."

  "$PYTHON_CMD" -m venv "$VENV_DIR"

fi



VENV_PYTHON="$VENV_DIR/bin/python"

PIP_MARKER="$VENV_DIR/.pip_installed_breeze_store"

STORE_ROOT="${BGX_TTS_STORE_ROOT:-}"

WHEELHOUSE=""



if [[ -z "$STORE_ROOT" ]]; then

  write_bootstrap error "BGX_TTS_STORE_ROOT not set - run Rust predownload_tts_artifacts first"

  exit 1

fi

WHEELHOUSE="$STORE_ROOT/wheelhouse"

# manifest.remoteArtifacts must exist (Rust orchestrator SSOT)
"$VENV_PYTHON" -c "import json; json.load(open('$MANIFEST'))['remoteArtifacts']" >/dev/null



if [[ ! -f "$PIP_MARKER" ]]; then

  if [[ ! -d "$WHEELHOUSE" ]]; then

    write_bootstrap manual_required "wheelhouse missing at $WHEELHOUSE - predownload via client first"

    exit 1

  fi

  write_bootstrap pip_install "pip install --no-index --find-links $WHEELHOUSE"

  bgx_probe_needs_install "pip install from wheelhouse (requirements hash changed or first install)"
  if ! bgx_pip_offline "$VENV_PYTHON" -m pip install --no-index --find-links "$WHEELHOUSE" -r "$REQUIREMENTS"; then

    pip_exit=$?

    write_bootstrap error "pip install failed exit=$pip_exit. See $STDERR_LOG"

    exit "$pip_exit"

  fi

  echo ok >"$PIP_MARKER"

  write_bootstrap pip_done "pip dependencies installed from local wheelhouse"

else

  write_bootstrap pip_skip "pip dependencies already installed; skipping"

fi



echo "$$" >"$PID_FILE"

write_bootstrap launch "Starting sidecar_server.py..."

bgx_probe_ready
exec "$VENV_PYTHON" "$SERVER_PY" 1>&2

