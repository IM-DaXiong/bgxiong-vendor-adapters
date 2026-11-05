#!/usr/bin/env python3
"""Breeze TTS 2 HTTP sidecar — JSON wrapper over frozen upstream (plan §2.6).

Bind 127.0.0.1 only. Shared TtsSidecarManager allocates the port.
Does not modify VoxCPM/IndexTTS. Wrapper is the only conversion layer.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
import sys
import threading
import time
import uuid
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any, AsyncIterator, Optional

import uvicorn
from fastapi import FastAPI, File, HTTPException, Request, Response, UploadFile
from fastapi.responses import JSONResponse, StreamingResponse
from pydantic import BaseModel, ConfigDict, Field
from starlette.concurrency import run_in_threadpool

RUNTIME_ROOT = Path(__file__).resolve().parent


def _resolve_shared_dir(runtime_root: Path) -> Path:
    bundled = runtime_root / "_shared"
    if (bundled / "bgx_sidecar_common.py").is_file():
        return bundled
    # Dev tree: runtimes/tts/_shared (two parents up from breeze/ -> tts/)
    dev_tts_shared = runtime_root.parent / "_shared"
    if (dev_tts_shared / "bgx_sidecar_common.py").is_file():
        return dev_tts_shared
    # Alternate: runtimes/_shared
    alt = runtime_root.parent.parent / "_shared"
    if (alt / "bgx_sidecar_common.py").is_file():
        return alt
    return bundled


SHARED_DIR = _resolve_shared_dir(RUNTIME_ROOT)
BOOTSTRAP_JSONL = RUNTIME_ROOT / "sidecar.bootstrap.jsonl"
MANIFEST_PATH = RUNTIME_ROOT / "manifest.json"
HOST = "127.0.0.1"
MODEL_ID_DEFAULT = "BreezeBlue/Breeze-TTS-2"


TMP_REF_DIR = RUNTIME_ROOT / "tmp_refs"
TMP_REF_TTL_SEC = int(os.environ.get("BGX_BREEZE_TMP_REF_TTL_SEC", "3600"))
_ALLOWED_TMP_MIME = frozenset(
    {
        "audio/wav",
        "audio/x-wav",
        "audio/wave",
        "audio/mpeg",
        "audio/mp3",
        "audio/flac",
        "audio/ogg",
        "audio/webm",
        "audio/mp4",
        "audio/aac",
        "application/octet-stream",
    }
)


def _ensure_tmp_ref_dir() -> Path:
    TMP_REF_DIR.mkdir(parents=True, exist_ok=True)
    return TMP_REF_DIR


def _cleanup_expired_tmp_refs(now: Optional[float] = None) -> None:
    root = _ensure_tmp_ref_dir()
    now_v = time.time() if now is None else now
    for meta_path in root.glob("*.meta.json"):
        try:
            meta = json.loads(meta_path.read_text(encoding="utf-8"))
            expires = float(meta.get("expiresAtUnix", 0))
            if expires and expires < now_v:
                audio = root / str(meta.get("fileName") or "")
                if audio.is_file():
                    audio.unlink(missing_ok=True)
                meta_path.unlink(missing_ok=True)
        except Exception:
            log.debug("tmp_ref cleanup skip %s", meta_path, exc_info=True)


def _resolve_tmp_audio_path(temp_id: str) -> Path:
    if not isinstance(temp_id, str) or not temp_id or any(ch in temp_id for ch in ("/", chr(92), "..")):
        raise HTTPException(status_code=400, detail="invalid tempId")
    # tempId is hex; file stored as {tempId}{ext}
    root = _ensure_tmp_ref_dir().resolve()
    meta_path = root / f"{temp_id}.meta.json"
    if not meta_path.is_file():
        raise HTTPException(status_code=404, detail="temp reference not found")
    meta = json.loads(meta_path.read_text(encoding="utf-8"))
    expires = float(meta.get("expiresAtUnix", 0))
    if expires and expires < time.time():
        raise HTTPException(status_code=410, detail="temp reference expired")
    file_name = str(meta.get("fileName") or "")
    audio_path = (root / file_name).resolve()
    if root not in audio_path.parents and audio_path != root:
        raise HTTPException(status_code=400, detail="temp reference path escapes lease root")
    if not audio_path.is_file():
        raise HTTPException(status_code=404, detail="temp reference audio missing")
    return audio_path



if str(RUNTIME_ROOT) not in sys.path:
    sys.path.insert(0, str(RUNTIME_ROOT))
if str(SHARED_DIR) not in sys.path:
    sys.path.insert(0, str(SHARED_DIR))

from bgx_remote_fetch import ManualInstallRequired, ensure_artifacts  # noqa: E402
from bgx_sidecar_common import append_bootstrap, make_remote_progress_fn  # noqa: E402
from breeze_runtime import BreezeRuntime  # noqa: E402
from request_map import RequestMapError, map_speech_request, parse_runtime_options  # noqa: E402

_request_lock = threading.Lock()
_runtime = BreezeRuntime()

logging.basicConfig(
    level=logging.DEBUG if os.environ.get("BGX_TTS_SIDECAR_DEBUG") == "1" else logging.INFO,
    format="%(asctime)s [breeze-sidecar] %(levelname)s %(message)s",
)
log = logging.getLogger("breeze-sidecar")


class VoiceDesignBody(BaseModel):
    model_config = ConfigDict(extra="forbid")
    controlPrompt: Optional[str] = None


class ReferenceBody(BaseModel):
    model_config = ConfigDict(extra="forbid")
    audioPath: str
    promptText: Optional[str] = None
    styleControl: Optional[str] = None


class DepthDecoderBody(BaseModel):
    model_config = ConfigDict(extra="forbid")
    doSample: Optional[bool] = None
    temperature: Optional[float] = None
    topK: Optional[int] = None
    topP: Optional[float] = None


class ExecutionBody(BaseModel):
    model_config = ConfigDict(extra="forbid")
    delivery: Optional[str] = "complete"
    seed: Optional[int] = None
    guidanceScale: Optional[float] = None
    maxOutputTokens: Optional[int] = None
    maxContextTokens: Optional[int] = None
    temperature: Optional[float] = None
    topK: Optional[int] = None
    topP: Optional[float] = None
    doSample: Optional[bool] = None
    repetitionPenalty: Optional[float] = None
    depthDecoder: Optional[DepthDecoderBody] = None
    collectTiming: Optional[bool] = None
    runtimeProfile: Optional[dict[str, Any]] = None


class SpeechRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str = MODEL_ID_DEFAULT
    input: str
    mode: str = "plain"
    voiceDesign: Optional[VoiceDesignBody] = None
    reference: Optional[ReferenceBody] = None
    execution: Optional[ExecutionBody] = None
    runtimeOptions: Optional[dict[str, Any]] = None
    requestId: Optional[str] = None


class BatchItem(BaseModel):
    model_config = ConfigDict(extra="forbid")
    id: str
    input: str


class BatchSpeechRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str = MODEL_ID_DEFAULT
    mode: str = "plain"
    voiceDesign: Optional[VoiceDesignBody] = None
    reference: Optional[ReferenceBody] = None
    execution: Optional[ExecutionBody] = None
    runtimeOptions: Optional[dict[str, Any]] = None
    items: list[BatchItem] = Field(min_length=1)


def load_manifest() -> dict[str, Any]:
    return json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))


def bootstrap_artifacts() -> None:
    ensure_artifacts(load_manifest(), RUNTIME_ROOT, make_remote_progress_fn(BOOTSTRAP_JSONL))


def _env_runtime_options() -> dict[str, Any]:
    raw = os.environ.get("BGX_BREEZE_RUNTIME_OPTIONS", "").strip()
    if not raw:
        return parse_runtime_options(None)
    try:
        return parse_runtime_options(json.loads(raw))
    except Exception as exc:
        raise RuntimeError(f"Invalid BGX_BREEZE_RUNTIME_OPTIONS: {exc}") from exc


def load_model() -> None:
    append_bootstrap(BOOTSTRAP_JSONL, "model_load", "Loading Breeze TTS 2 from checkpoints...")
    options = _env_runtime_options()
    _runtime.load(options)
    append_bootstrap(
        BOOTSTRAP_JSONL,
        "model_ready",
        f"Model loaded (sample_rate={_runtime.sample_rate} fingerprint={_runtime.profile_fingerprint[:12]})",
    )


@asynccontextmanager
async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
    append_bootstrap(BOOTSTRAP_JSONL, "server_start", "Starting Breeze HTTP server...")
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
    try:
        await loop.run_in_executor(None, load_model)
    except Exception:
        log.exception("load_model failed")
        raise
    append_bootstrap(BOOTSTRAP_JSONL, "ready", "Sidecar ready for synthesis requests")
    yield


app = FastAPI(title="Breeze TTS 2 Sidecar", version="1.0.0", lifespan=lifespan)


@app.get("/health")
async def health() -> JSONResponse:
    ready = _runtime.loaded
    body: dict[str, Any] = {
        "status": "ok" if ready else "loading",
        "modelLoaded": ready,
        "engine": "breeze",
        "version": "1.0.0",
    }
    if ready:
        body["sampleRate"] = _runtime.sample_rate
        body["profileFingerprint"] = _runtime.profile_fingerprint
        body["requestedProfile"] = _runtime.requested_profile
        body["effectiveProfile"] = _runtime.effective_profile
    return JSONResponse(body)


@app.get("/v1/models")
async def list_models() -> JSONResponse:
    return JSONResponse(
        {
            "object": "list",
            "data": [
                {
                    "id": MODEL_ID_DEFAULT,
                    "object": "model",
                    "owned_by": "BreezeBlue",
                }
            ],
        }
    )


@app.get("/v1/model")
async def model_info() -> JSONResponse:
    if not _runtime.loaded:
        raise HTTPException(status_code=503, detail="model not loaded")
    info = _runtime.model_info
    info = {
        **info,
        "engine": "breeze",
        "sampleRate": _runtime.sample_rate,
        "profileFingerprint": _runtime.profile_fingerprint,
        "requestedProfile": _runtime.requested_profile,
        "effectiveProfile": _runtime.effective_profile,
        "attnImplementation": "eager",
    }
    return JSONResponse(info)


@app.get("/v1/requests/{request_id}")
async def request_status(request_id: str) -> JSONResponse:
    status = _runtime.get_status(request_id)
    if status is None:
        return JSONResponse({"requestId": request_id, "state": "unknown"}, status_code=404)
    return JSONResponse(
        {
            "requestId": status.request_id,
            "state": status.state,
            "sampleCount": status.sample_count,
            "pcmBytes": status.pcm_bytes,
            "digest": status.digest,
            "error": status.error,
            "startedAt": status.started_at,
            "finishedAt": status.finished_at,
        }
    )


def _body_to_dict(req: SpeechRequest) -> dict[str, Any]:
    return req.model_dump(exclude_none=True)


def _map_or_400(body: dict[str, Any]) -> dict[str, Any]:
    try:
        return map_speech_request(body)
    except RequestMapError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc



@app.post("/v1/audio/reference-temp")
async def upload_reference_temp(file: UploadFile = File(...)) -> JSONResponse:
    """Accept SDK-authorized audio bytes into a companion-scoped temp lease.

    Host/adapters must not invent arbitrary absolute paths. The returned audioPath
    is only meaningful inside this sidecar process and is cleaned on expiry/delete.
    """
    _cleanup_expired_tmp_refs()
    filename = file.filename or "reference.bin"
    # basename only
    base = Path(filename).name
    if not base or base in {".", ".."} or "/" in base or chr(92) in base:
        raise HTTPException(status_code=400, detail="invalid file name")
    mime = (file.content_type or "application/octet-stream").split(";")[0].strip().lower()
    if mime not in _ALLOWED_TMP_MIME and not mime.startswith("audio/"):
        raise HTTPException(status_code=400, detail=f"unsupported audio mime: {mime}")

    data = await file.read()
    if not data or len(data) < 64:
        raise HTTPException(status_code=400, detail="audio payload too small")
    if len(data) > 64 * 1024 * 1024:
        raise HTTPException(status_code=400, detail="audio payload too large")

    temp_id = uuid.uuid4().hex
    suffix = Path(base).suffix.lower()
    if suffix not in {".wav", ".mp3", ".flac", ".ogg", ".webm", ".m4a", ".aac", ".bin"}:
        # keep original extension when plausible; otherwise .bin
        if len(suffix) > 8 or not suffix.startswith("."):
            suffix = ".bin"
    stored_name = f"{temp_id}{suffix}"
    root = _ensure_tmp_ref_dir()
    audio_path = root / stored_name
    meta_path = root / f"{temp_id}.meta.json"
    expires = time.time() + TMP_REF_TTL_SEC
    audio_path.write_bytes(data)
    meta = {
        "tempId": temp_id,
        "fileName": stored_name,
        "originalFileName": base,
        "mime": mime,
        "byteLength": len(data),
        "expiresAtUnix": expires,
        "audioPath": str(audio_path.resolve()),
    }
    meta_path.write_text(json.dumps(meta, ensure_ascii=False, indent=2), encoding="utf-8")
    return JSONResponse(
        {
            "tempId": temp_id,
            "audioPath": str(audio_path.resolve()),
            "fileName": stored_name,
            "originalFileName": base,
            "mime": mime,
            "byteLength": len(data),
            "expiresAtUnix": expires,
        }
    )


@app.delete("/v1/audio/reference-temp/{temp_id}")
async def delete_reference_temp(temp_id: str) -> JSONResponse:
    _cleanup_expired_tmp_refs()
    if not temp_id or any(ch in temp_id for ch in ("/", chr(92), "..")):
        raise HTTPException(status_code=400, detail="invalid tempId")
    root = _ensure_tmp_ref_dir()
    meta_path = root / f"{temp_id}.meta.json"
    if not meta_path.is_file():
        return JSONResponse({"tempId": temp_id, "deleted": False, "reason": "not_found"})
    try:
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
        audio = root / str(meta.get("fileName") or "")
        if audio.is_file():
            audio.unlink(missing_ok=True)
        meta_path.unlink(missing_ok=True)
    except Exception as exc:
        raise HTTPException(status_code=500, detail=f"failed to delete temp reference: {exc}") from exc
    return JSONResponse({"tempId": temp_id, "deleted": True})


@app.post("/v1/audio/speech")
async def create_speech(req: SpeechRequest, raw: Request) -> Response:
    if not _runtime.loaded:
        raise HTTPException(status_code=503, detail="模型尚未加载完成")

    if not _request_lock.acquire(blocking=False):
        raise HTTPException(
            status_code=409,
            detail="An inference request is already running.",
            headers={"Retry-After": "5"},
        )

    request_id = req.requestId or f"api-{uuid.uuid4().hex}"
    body = _body_to_dict(req)
    body["requestId"] = request_id
    mapped = _map_or_400(body)
    delivery = mapped["execution"]["delivery"]

    def cleanup() -> None:
        _request_lock.release()

    if delivery == "stream":

        def stream_body():
            try:
                gen = _runtime.synthesize_pcm_chunks(mapped=mapped)
                try:
                    while True:
                        yield next(gen)
                except StopIteration:
                    return
            finally:
                cleanup()

        return StreamingResponse(
            stream_body(),
            media_type="audio/pcm",
            headers={
                "X-Sample-Rate": str(_runtime.sample_rate),
                "X-Sample-Format": "s16le",
                "X-Request-Id": request_id,
                "Cache-Control": "no-store",
            },
        )

    # complete WAV
    try:
        wav, status = await run_in_threadpool(_runtime.synthesize_wav, mapped)
    except Exception as exc:
        cleanup()
        log.exception("synthesize failed")
        raise HTTPException(status_code=500, detail=f"Breeze 合成失败: {exc}") from exc
    cleanup()
    return Response(
        content=wav,
        media_type="audio/wav",
        headers={
            "X-Request-Id": request_id,
            "X-Sample-Rate": str(_runtime.sample_rate),
            "Cache-Control": "no-store",
        },
    )


@app.post("/v1/audio/speech/batch")
async def create_speech_batch(req: BatchSpeechRequest) -> Response:
    if not _runtime.loaded:
        raise HTTPException(status_code=503, detail="模型尚未加载完成")
    if not _request_lock.acquire(blocking=False):
        raise HTTPException(
            status_code=409,
            detail="An inference request is already running.",
            headers={"Retry-After": "5"},
        )

    # Build shared single-item body from first text for mapping; replace text list later.
    first = req.items[0]
    shared_body: dict[str, Any] = {
        "model": req.model,
        "input": first.input,
        "mode": req.mode,
        "requestId": first.id,
    }
    if req.voiceDesign is not None:
        shared_body["voiceDesign"] = req.voiceDesign.model_dump(exclude_none=True)
    if req.reference is not None:
        shared_body["reference"] = req.reference.model_dump(exclude_none=True)
    if req.execution is not None:
        shared_body["execution"] = req.execution.model_dump(exclude_none=True)
    if req.runtimeOptions is not None:
        shared_body["runtimeOptions"] = req.runtimeOptions
    # Force complete for batch.
    shared_body.setdefault("execution", {})
    if isinstance(shared_body["execution"], dict):
        shared_body["execution"]["delivery"] = "complete"

    try:
        mapped = _map_or_400(shared_body)
        texts = [item.input for item in req.items]
        ids = [item.id for item in req.items]
        # Ensure unique ids
        if len(set(ids)) != len(ids):
            raise HTTPException(status_code=400, detail="batch item ids must be unique")

        def run_batch():
            return _runtime.synthesize_batch_pcm(texts=texts, shared_mapped=mapped)

        segments, _ = await run_in_threadpool(run_batch)
    except HTTPException:
        _request_lock.release()
        raise
    except RequestMapError as exc:
        _request_lock.release()
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    except Exception as exc:
        _request_lock.release()
        log.exception("batch failed")
        raise HTTPException(status_code=500, detail=f"Breeze batch 失败: {exc}") from exc
    _request_lock.release()

    return Response(
        content=b"".join(segments),
        media_type="application/octet-stream",
        headers={
            "X-Segment-Bytes": ",".join(str(len(s)) for s in segments),
            "X-Segment-Ids": ",".join(ids),
            "X-Sample-Rate": str(_runtime.sample_rate),
            "X-Sample-Format": "s16le",
            "Cache-Control": "no-store",
        },
    )


def main() -> None:
    port_str = os.environ.get("BGX_TTS_SIDECAR_PORT", "18200")
    try:
        port = int(port_str)
    except ValueError as e:
        print(f"Invalid BGX_TTS_SIDECAR_PORT: {port_str}", file=sys.stderr)
        raise SystemExit(1) from e
    append_bootstrap(BOOTSTRAP_JSONL, "init", f"Sidecar Python process started port={port}")
    uvicorn.run(app, host=HOST, port=port, log_level="info", access_log=False)


if __name__ == "__main__":
    main()
