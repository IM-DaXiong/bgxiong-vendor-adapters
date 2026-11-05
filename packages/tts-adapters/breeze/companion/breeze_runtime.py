"""Breeze TTS 2 runtime wrapper over frozen upstream snapshot.

Loads tokenizer/model/audio_tokenizer from local checkpoints, drives
FastBreezeStreamingRuntime for single-request complete/stream, and
native eager batch. Does not copy inference core; imports from
runtimes/tts/breeze/upstream/ on sys.path.
"""

from __future__ import annotations

import hashlib
import io
import json
import logging
import os
import threading
import time
import uuid
from contextlib import closing, contextmanager
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any, Callable, Generator, Iterator, Mapping, Optional

import numpy as np

from request_map import effective_fast_flags

log = logging.getLogger("breeze-runtime")

RUNTIME_ROOT = Path(__file__).resolve().parent
UPSTREAM_ROOT = RUNTIME_ROOT / "upstream"
FAST_CONFIG = UPSTREAM_ROOT / "configs" / "fast.json"
DEFAULT_SAMPLE_RATE = 24000
MAX_BATCH_TEXTS = 128


def ensure_upstream_on_path() -> Path:
    import sys

    root = str(UPSTREAM_ROOT)
    if root not in sys.path:
        sys.path.insert(0, root)
    return UPSTREAM_ROOT


def resolve_store_root() -> Path:
    env = os.environ.get("BGX_TTS_STORE_ROOT", "").strip()
    if env:
        return Path(env)
    return RUNTIME_ROOT.parent / "_store"


def resolve_checkpoints_dir() -> Path:
    override = os.environ.get("BGX_BREEZE_CHECKPOINTS", "").strip()
    if override:
        return Path(override)
    return resolve_store_root() / "checkpoints"


def _pcm16(audio: np.ndarray) -> bytes:
    audio = np.asarray(audio, dtype=np.float32)
    audio = np.clip(audio, -1.0, 1.0)
    return (audio * 32767.0).astype("<i2", copy=False).tobytes()


def _wav_bytes(pcm: bytes, sample_rate: int) -> bytes:
    import soundfile as sf

    samples = np.frombuffer(pcm, dtype="<i2").astype(np.float32) / 32767.0
    buf = io.BytesIO()
    sf.write(buf, samples, sample_rate, format="WAV", subtype="PCM_16")
    return buf.getvalue()


@dataclass
class RequestStatus:
    request_id: str
    state: str  # running|complete|failed|cancelled
    sample_count: int = 0
    pcm_bytes: int = 0
    digest: str = ""
    error: Optional[str] = None
    started_at: float = 0.0
    finished_at: Optional[float] = None


class BreezeRuntime:
    """Owns one loaded model + profile fingerprint for a sidecar process."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._loaded = False
        self._tokenizer = None
        self._model = None
        self._audio_tokenizer = None
        self._stream_runtime = None
        self._batch_head_weights = None
        self._batch_depth_decoder = None
        self._sample_rate = DEFAULT_SAMPLE_RATE
        self._model_info: dict[str, Any] = {}
        self._checkpoints: Optional[Path] = None
        self._requested_profile: dict[str, Any] = {}
        self._effective_profile: dict[str, bool] = {}
        self._profile_fingerprint = ""
        self._statuses: dict[str, RequestStatus] = {}

    @property
    def loaded(self) -> bool:
        return self._loaded

    @property
    def sample_rate(self) -> int:
        return int(self._sample_rate)

    @property
    def model_info(self) -> dict[str, Any]:
        return dict(self._model_info)

    @property
    def profile_fingerprint(self) -> str:
        return self._profile_fingerprint

    @property
    def requested_profile(self) -> dict[str, Any]:
        return dict(self._requested_profile)

    @property
    def effective_profile(self) -> dict[str, bool]:
        return dict(self._effective_profile)

    def get_status(self, request_id: str) -> Optional[RequestStatus]:
        return self._statuses.get(request_id)

    def load(self, runtime_options: Mapping[str, Any] | None = None) -> None:
        if self._loaded:
            return
        ensure_upstream_on_path()
        from breeze_infer.api import build_model_info  # type: ignore[import-not-found]
        from breeze_infer.batch import BatchDepthDecoder  # type: ignore[import-not-found]
        from breeze_infer.runtime import (  # type: ignore[import-not-found]
            load_runtime,
            resolve_device,
            update_generation_config_for_breeze,
        )
        from models.fast_streaming import (  # type: ignore[import-not-found]
            FastBreezeStreamingRuntime,
            FastStreamingConfig,
        )
        from models.warmup_profile import load_warmup_profile  # type: ignore[import-not-found]

        ckpt = resolve_checkpoints_dir()
        if not ckpt.is_dir():
            raise FileNotFoundError(f"Breeze checkpoints not found: {ckpt}")
        options = dict(runtime_options or {})
        effective = effective_fast_flags(options)
        device = resolve_device()
        if not str(device).startswith("cuda"):
            raise RuntimeError(
                f"Breeze S0 requires CUDA; resolve_device returned {device!r}"
            )

        log.info("loading Breeze from %s on %s", ckpt, device)
        tokenizer, model, audio_tokenizer = load_runtime(
            ckpt,
            device=device,
            attn_implementation="eager",
        )
        update_generation_config_for_breeze(model)

        # Snapshot projection heads for batch weight swap (upstream api.py).
        import torch

        heads = (model.lm_head, model.depth_decoder.codebooks_head)
        batch_head_weights = {
            head: torch.nn.Parameter(
                head.weight.detach().to(device="cpu", copy=True), requires_grad=False
            )
            for head in heads
        }
        batch_depth_decoder = BatchDepthDecoder(model, max_batch_size=MAX_BATCH_TEXTS)

        config = FastStreamingConfig(
            max_new_tokens=1500,
            max_seq_len=2048,
            fast_all=options.get("fastAll", None),
            fast_text_encoder=effective["fastTextEncoder"],
            fast_backbone_prefill=effective["fastBackbonePrefill"],
            fast_backbone_decode=effective["fastBackboneDecode"],
            fast_depth_decoder=effective["fastDepthDecoder"],
            fast_codec=effective["fastCodec"],
            repetition_penalty=1.1,
        )
        stream_runtime = FastBreezeStreamingRuntime(
            model, audio_tokenizer, config, tokenizer=tokenizer
        )
        if stream_runtime.fast_enabled:
            profile = load_warmup_profile(FAST_CONFIG)
            profile = replace(profile, codec_chunk_frames=stream_runtime.codec_chunk_frames)
            manifest = stream_runtime.warmup_from_profile(profile)
            log.info("fast warmup total_elapsed_ms=%.2f", manifest["total_elapsed_ms"])

        info = build_model_info(ckpt)
        fingerprint = hashlib.sha256(
            json.dumps(
                {"requested": options, "effective": effective, "ckpt": str(ckpt)},
                sort_keys=True,
                ensure_ascii=False,
            ).encode("utf-8")
        ).hexdigest()

        self._tokenizer = tokenizer
        self._model = model
        self._audio_tokenizer = audio_tokenizer
        self._stream_runtime = stream_runtime
        self._batch_head_weights = batch_head_weights
        self._batch_depth_decoder = batch_depth_decoder
        self._sample_rate = int(stream_runtime.sample_rate)
        self._model_info = info
        self._checkpoints = ckpt
        self._requested_profile = options
        self._effective_profile = effective
        self._profile_fingerprint = fingerprint
        self._loaded = True
        log.info(
            "model ready sample_rate=%s digest=%s fingerprint=%s",
            self._sample_rate,
            info.get("model_digest"),
            fingerprint[:12],
        )

    def _apply_generation_overrides(self, execution: Mapping[str, Any]) -> Callable[[], None]:
        """Patch model generation_config; return restore callback."""
        assert self._model is not None
        model = self._model
        depth = execution["depthDecoder"]
        previous = {
            "do_sample": model.generation_config.do_sample,
            "temperature": model.generation_config.temperature,
            "top_k": model.generation_config.top_k,
            "top_p": model.generation_config.top_p,
        }
        previous_depth = {
            "do_sample": model.depth_decoder.generation_config.do_sample,
            "temperature": model.depth_decoder.generation_config.temperature,
            "top_k": model.depth_decoder.generation_config.top_k,
            "top_p": model.depth_decoder.generation_config.top_p,
        }
        from breeze_infer.runtime import update_generation_config_for_breeze  # type: ignore[import-not-found]

        update_generation_config_for_breeze(
            model,
            {
                "do_sample": execution["doSample"],
                "temperature": execution["temperature"],
                "top_k": execution["topK"],
                "top_p": execution["topP"],
                "depth_decoder_do_sample": depth["doSample"],
                "depth_decoder_temperature": depth["temperature"],
                "depth_decoder_top_k": depth["topK"],
                "depth_decoder_top_p": depth["topP"],
            },
        )

        def restore() -> None:
            vars(model.generation_config).update(previous)
            vars(model.depth_decoder.generation_config).update(previous_depth)

        return restore

    def _prepare_inputs(
        self,
        upstream_request: Mapping[str, Any],
        *,
        template_name: str,
        guidance_scale: float,
    ):
        ensure_upstream_on_path()
        from breeze_infer.templates import get_template, prepare_inputs  # type: ignore[import-not-found]

        assert self._tokenizer is not None
        assert self._audio_tokenizer is not None
        assert self._model is not None
        return prepare_inputs(
            self._tokenizer,
            self._audio_tokenizer,
            self._model,
            [dict(upstream_request)],
            get_template(template_name),
            guidance_scale=guidance_scale,
            guidance_scale_ref=None,
            guidance_scale_ins=None,
        )

    def synthesize_pcm_chunks(
        self,
        *,
        mapped: Mapping[str, Any],
        on_status: Optional[Callable[[RequestStatus], None]] = None,
    ) -> Generator[bytes, None, RequestStatus]:
        """Yield PCM s16le chunks; return final status."""
        if not self._loaded or self._stream_runtime is None:
            raise RuntimeError("Breeze runtime not loaded")

        ensure_upstream_on_path()
        from breeze_infer.runtime import set_all_seeds  # type: ignore[import-not-found]
        from models.fast_streaming import FastStreamingConfig  # type: ignore[import-not-found]

        execution = mapped["execution"]
        request_id = mapped.get("request_id") or f"api-{uuid.uuid4().hex}"
        status = RequestStatus(
            request_id=request_id, state="running", started_at=time.time()
        )
        self._statuses[request_id] = status
        if on_status:
            on_status(status)

        # FastStreamingConfig is frozen; replace per-request limits on the runtime.
        cfg = self._stream_runtime.config
        if isinstance(cfg, FastStreamingConfig):
            from dataclasses import replace as _replace_cfg

            self._stream_runtime.config = _replace_cfg(
                cfg,
                max_new_tokens=int(execution["maxOutputTokens"]),
                max_seq_len=int(execution["maxContextTokens"]),
                repetition_penalty=float(execution["repetitionPenalty"]),
                collect_timing=bool(execution["collectTiming"]),
            )

        restore = self._apply_generation_overrides(execution)
        digest = hashlib.sha256()
        try:
            inputs = self._prepare_inputs(
                mapped["upstream_request"],
                template_name=mapped["template_name"],
                guidance_scale=float(execution["guidanceScale"]),
            )
            set_all_seeds(int(execution["seed"]))

            def iter_chunks():
                set_all_seeds(int(execution["seed"]))
                yield from self._stream_runtime.iter_audio_chunks(
                    inputs,
                    request_id=request_id,
                    seed=int(execution["seed"]),
                )

            with closing(iter_chunks()) as chunks:
                for chunk in chunks:
                    pcm = _pcm16(chunk.audio)
                    if not pcm:
                        continue
                    digest.update(pcm)
                    status.pcm_bytes += len(pcm)
                    status.sample_count = status.pcm_bytes // 2
                    yield pcm
            status.state = "complete"
            status.digest = digest.hexdigest()
            status.finished_at = time.time()
        except GeneratorExit:
            status.state = "cancelled"
            status.error = "cancelled"
            status.finished_at = time.time()
            raise
        except Exception as exc:
            status.state = "failed"
            status.error = str(exc)
            status.finished_at = time.time()
            raise
        finally:
            restore()
            if on_status:
                on_status(status)
        return status

    def synthesize_wav(self, mapped: Mapping[str, Any]) -> tuple[bytes, RequestStatus]:
        pcm_parts: list[bytes] = []
        gen = self.synthesize_pcm_chunks(mapped=mapped)
        try:
            while True:
                pcm_parts.append(next(gen))
        except StopIteration as stop:
            status = stop.value
        wav = _wav_bytes(b"".join(pcm_parts), self.sample_rate)
        return wav, status

    def synthesize_batch_pcm(
        self,
        *,
        texts: list[str],
        shared_mapped: Mapping[str, Any],
    ) -> tuple[list[bytes], list[str]]:
        """Native eager batch; returns PCM segments + item ids."""
        if not self._loaded:
            raise RuntimeError("Breeze runtime not loaded")
        if not texts:
            raise ValueError("batch texts must be non-empty")
        if len(texts) > MAX_BATCH_TEXTS:
            raise ValueError(f"batch size {len(texts)} exceeds {MAX_BATCH_TEXTS}")

        ensure_upstream_on_path()
        import torch
        from breeze_infer.batch import eager_batch_modules  # type: ignore[import-not-found]
        from breeze_infer.runtime import set_all_seeds  # type: ignore[import-not-found]
        from breeze_infer.templates import get_template, prepare_inputs  # type: ignore[import-not-found]
        from models.logits_process import (  # type: ignore[import-not-found]
            GeneratedTokenRepetitionPenaltyLogitsProcessor,
        )
        from transformers.generation.logits_process import LogitsProcessorList

        execution = shared_mapped["execution"]
        base = dict(shared_mapped["upstream_request"])
        requests = []
        ids = []
        for index, text in enumerate(texts):
            if not isinstance(text, str) or text.strip() == "":
                raise ValueError(f"batch item {index} text must be non-empty")
            req = dict(base)
            req_id = f"batch-{index}"
            req["id"] = req_id
            req["text"] = text
            requests.append(req)
            ids.append(req_id)

        template_name = shared_mapped["template_name"]
        # Re-derive template from first request fields (instruction/ref may set edit).
        from breeze_infer.templates import select_template_name  # type: ignore[import-not-found]

        template_name = select_template_name(requests[0])
        restore = self._apply_generation_overrides(execution)
        try:
            set_all_seeds(int(execution["seed"]))
            inputs = prepare_inputs(
                self._tokenizer,
                self._audio_tokenizer,
                self._model,
                requests,
                get_template(template_name),
                guidance_scale=float(execution["guidanceScale"]),
                guidance_scale_ref=None,
                guidance_scale_ins=None,
            )
            logits_processor = LogitsProcessorList(
                [
                    GeneratedTokenRepetitionPenaltyLogitsProcessor(
                        float(execution["repetitionPenalty"])
                    )
                ]
            )

            @contextmanager
            def swapped_weights(weights):
                originals = {module: module.weight for module in weights}
                try:
                    for module, weight in weights.items():
                        module.weight = (
                            weight
                            if weight.device == module.weight.device
                            else torch.nn.Parameter(
                                weight.to(module.weight.device), requires_grad=False
                            )
                        )
                    yield
                finally:
                    for module, weight in originals.items():
                        module.weight = weight

            cfg_scale = float(execution["guidanceScale"])
            with eager_batch_modules(self._model):
                if cfg_scale != 1.0:
                    self._batch_depth_decoder.prepare(len(texts))
                set_all_seeds(int(execution["seed"]))
                with (
                    torch.inference_mode(),
                    swapped_weights(self._batch_head_weights),
                    self._batch_depth_decoder.bound(self._model),
                ):
                    audio = self._model.generate(
                        **inputs,
                        output_audio=True,
                        audio_tokenizer=self._audio_tokenizer,
                        logits_processor=logits_processor,
                        max_new_tokens=int(execution["maxOutputTokens"]),
                    )
            if len(audio) != len(texts):
                raise RuntimeError(
                    f"Batch size mismatch: sent {len(texts)}, got {len(audio)}"
                )
            segments = [
                _pcm16(segment.detach().float().cpu().numpy().reshape(-1))
                for segment in audio
            ]
            return segments, ids
        finally:
            restore()
