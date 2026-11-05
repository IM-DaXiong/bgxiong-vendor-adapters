#!/usr/bin/env python3
"""VoxCPM2 TTS HTTP sidecar — OpenAI-compatible /v1/audio/speech (SSOT §2.4)."""

from __future__ import annotations

import asyncio
import io
import json
import logging
import os
import sys
import time
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any, Literal, Optional

import soundfile as sf
import uvicorn
from fastapi import FastAPI, HTTPException, Response
from fastapi.responses import JSONResponse
from pydantic import BaseModel


def _resolve_shared_dir(runtime_root: Path) -> Path:
    """Launcher zip bundles `_shared/` beside sidecar scripts; dev tree uses `tts/_shared`."""
    bundled = runtime_root / "_shared"
    if (bundled / "bgx_sidecar_common.py").is_file():
        return bundled
    dev_tts_shared = runtime_root.parent.parent / "_shared"
    if (dev_tts_shared / "bgx_sidecar_common.py").is_file():
        return dev_tts_shared
    if (bundled / "bgx_remote_fetch.py").is_file():
        return bundled
    if (dev_tts_shared / "bgx_remote_fetch.py").is_file():
        return dev_tts_shared
    return bundled


RUNTIME_ROOT = Path(__file__).resolve().parent
SHARED_DIR = _resolve_shared_dir(RUNTIME_ROOT)
BOOTSTRAP_JSONL = RUNTIME_ROOT / "sidecar.bootstrap.jsonl"
MANIFEST_PATH = RUNTIME_ROOT / "manifest.json"
MODEL_ID = "openbmb/VoxCPM2"
HOST = "127.0.0.1"


def _resolve_store_root() -> Path:
    env = os.environ.get("BGX_TTS_STORE_ROOT", "").strip()
    if env:
        return Path(env)
    return RUNTIME_ROOT.parent / "_store"


CHECKPOINTS_DIR = _resolve_store_root() / "checkpoints"

if str(SHARED_DIR) not in sys.path:
    sys.path.insert(0, str(SHARED_DIR))

from bgx_remote_fetch import ManualInstallRequired, ensure_artifacts  # noqa: E402
from bgx_sidecar_common import append_bootstrap, make_remote_progress_fn  # noqa: E402

_request_lock = asyncio.Lock()
_model = None
_sample_rate: Optional[int] = None

logging.basicConfig(
    level=logging.DEBUG if os.environ.get("BGX_TTS_SIDECAR_DEBUG") == "1" else logging.INFO,
    format="%(asctime)s [voxcpm-sidecar] %(levelname)s %(message)s",
)
log = logging.getLogger("voxcpm-sidecar")


class VoiceDesignBody(BaseModel):
    controlPrompt: Optional[str] = None


class ReferenceBody(BaseModel):
    audioPath: str
    promptText: Optional[str] = None
    styleControl: Optional[str] = None


class InferenceBody(BaseModel):
    cfgValue: Optional[float] = 2.0
    inferenceTimesteps: Optional[int] = 10
    denoiseReference: Optional[bool] = False


class SpeechRequest(BaseModel):
    model: str = MODEL_ID
    input: str
    voice: Optional[str] = None
    mode: Literal["plain", "voice_design", "reference_clone"] = "plain"
    voiceDesign: Optional[VoiceDesignBody] = None
    reference: Optional[ReferenceBody] = None
    inference: Optional[InferenceBody] = None


def load_manifest() -> dict[str, Any]:
    return json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))


def bootstrap_artifacts() -> None:
    ensure_artifacts(load_manifest(), RUNTIME_ROOT, make_remote_progress_fn(BOOTSTRAP_JSONL))


def map_generate_kwargs(req: SpeechRequest) -> dict[str, Any]:
    """Map HTTP request -> VoxCPM.generate() kwargs (SSOT §2.3)."""
    text = req.input.strip()
    if not text:
        raise ValueError("input 不能为空")

    inf = req.inference or InferenceBody()
    kwargs: dict[str, Any] = {
        "text": text,
        "cfg_value": inf.cfgValue if inf.cfgValue is not None else 2.0,
        "inference_timesteps": inf.inferenceTimesteps if inf.inferenceTimesteps is not None else 10,
        "denoise": bool(inf.denoiseReference),
    }

    if req.mode == "plain":
        return kwargs

    if req.mode == "voice_design":
        if req.reference is not None:
            raise ValueError("voice_design 模式不得携带 reference")
        control = ""
        if req.voiceDesign and req.voiceDesign.controlPrompt:
            control = req.voiceDesign.controlPrompt.strip()
        if control:
            kwargs["text"] = f"({control}){text}"
        return kwargs

    if req.mode == "reference_clone":
        if req.voiceDesign is not None:
            raise ValueError("reference_clone 模式不得携带 voiceDesign")
        if req.reference is None or not req.reference.audioPath.strip():
            raise ValueError("reference_clone 需要 reference.audioPath")
        audio_path = Path(req.reference.audioPath)
        if not audio_path.is_file():
            raise FileNotFoundError(f"参考音频不存在: {audio_path}")

        prompt_text = (req.reference.promptText or "").strip()
        style = (req.reference.styleControl or "").strip()

        if prompt_text:
            kwargs["prompt_wav_path"] = str(audio_path)
            kwargs["prompt_text"] = prompt_text
            kwargs["reference_wav_path"] = str(audio_path)
        else:
            kwargs["reference_wav_path"] = str(audio_path)

        if style:
            kwargs["text"] = f"({style}){text}"
        return kwargs

    raise ValueError(f"不支持的 mode: {req.mode}")


def load_model() -> None:
    global _model, _sample_rate
    append_bootstrap(BOOTSTRAP_JSONL, "model_load", "Loading VoxCPM2 from checkpoints...")
    log.info("loading VoxCPM2 from %s", CHECKPOINTS_DIR)
    from voxcpm import VoxCPM

    _model = VoxCPM.from_pretrained(
        str(CHECKPOINTS_DIR),
        load_denoiser=False,
        local_files_only=True,
    )
    _sample_rate = int(_model.tts_model.sample_rate)
    append_bootstrap(BOOTSTRAP_JSONL, "model_ready", f"Model loaded (sample_rate={_sample_rate})")
    log.info("model loaded sample_rate=%s", _sample_rate)


@asynccontextmanager
async def lifespan(_app: FastAPI):
    append_bootstrap(BOOTSTRAP_JSONL, "server_start", "Starting HTTP server...")
    loop = asyncio.get_event_loop()
    try:
        await loop.run_in_executor(None, bootstrap_artifacts)
    except ManualInstallRequired as e:
        append_bootstrap(
            BOOTSTRAP_JSONL,
            "manual_required",
            e.payload.get("message", str(e)),
            **{k: v for k, v in e.payload.items() if k != "message"},
        )
        raise
    except Exception:
        log.exception("bootstrap_artifacts failed")
        raise
    try:
        await loop.run_in_executor(None, load_model)
    except Exception:
        log.exception("load_model failed")
        raise
    append_bootstrap(BOOTSTRAP_JSONL, "ready", "Sidecar ready for synthesis requests")
    yield


app = FastAPI(title="VoxCPM2 TTS Sidecar", version="2.0.9", lifespan=lifespan)


@app.get("/health")
async def health() -> JSONResponse:
    ready = _model is not None
    return JSONResponse({"status": "ok" if ready else "loading", "modelLoaded": ready})


@app.get("/v1/models")
async def list_models() -> JSONResponse:
    return JSONResponse(
        {
            "object": "list",
            "data": [{"id": MODEL_ID, "object": "model", "owned_by": "openbmb"}],
        }
    )


@app.post("/v1/audio/speech")
async def create_speech(req: SpeechRequest) -> Response:
    if _model is None:
        raise HTTPException(status_code=503, detail="模型尚未加载完成")

    if _request_lock.locked():
        raise HTTPException(
            status_code=503,
            detail="VoxCPM 正在合成，请稍后",
            headers={"Retry-After": "5"},
        )

    async with _request_lock:
        return await _synthesize(req)


async def _synthesize(req: SpeechRequest) -> Response:
    log.info(
        "synthesize mode=%s input_len=%d input=%r",
        req.mode,
        len(req.input),
        req.input,
    )
    try:
        gen_kwargs = map_generate_kwargs(req)
    except (ValueError, FileNotFoundError) as e:
        raise HTTPException(status_code=400, detail=str(e)) from e

    log.info(
        "generate text=%r mode=%s control_prompt=%r",
        gen_kwargs.get("text"),
        req.mode,
        req.voiceDesign.controlPrompt if req.voiceDesign else None,
    )

    loop = asyncio.get_event_loop()

    def _run() -> bytes:
        assert _model is not None
        wav = _model.generate(**gen_kwargs)
        buf = io.BytesIO()
        sf.write(buf, wav, _sample_rate or 24000, format="WAV")
        return buf.getvalue()

    started = time.perf_counter()
    try:
        wav_bytes = await loop.run_in_executor(None, _run)
    except Exception as e:
        log.exception("generate failed")
        raise HTTPException(status_code=500, detail=f"VoxCPM 合成失败: {e}") from e

    log.info(
        "synthesize ok bytes=%d elapsed_ms=%d",
        len(wav_bytes),
        int((time.perf_counter() - started) * 1000),
    )
    return Response(content=wav_bytes, media_type="audio/wav")


def main() -> None:
    port_str = os.environ.get("BGX_TTS_SIDECAR_PORT", "18100")
    try:
        port = int(port_str)
    except ValueError as e:
        print(f"Invalid BGX_TTS_SIDECAR_PORT: {port_str}", file=sys.stderr)
        raise SystemExit(1) from e

    append_bootstrap(BOOTSTRAP_JSONL, "init", f"Sidecar Python process started port={port}")
    uvicorn.run(app, host=HOST, port=port, log_level="info", access_log=False)


if __name__ == "__main__":
    main()
