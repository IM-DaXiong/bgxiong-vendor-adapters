"""Map product JSON speech request -> Breeze upstream request + execution options.

Unique mapping layer for runtimes/tts/breeze. Does not load weights.
Rejects unknown keys and VoxCPM/IndexTTS-only fields. Preserves text/
instruction/reference transcript verbatim (no trim-rewrite, no ASR).
"""

from __future__ import annotations

import math
from typing import Any, Mapping, MutableMapping, Optional

ALLOWED_TOP_KEYS = frozenset(
    {
        "model",
        "input",
        "mode",
        "voiceDesign",
        "reference",
        "execution",
        "runtimeOptions",
        "requestId",
    }
)
ALLOWED_VOICE_DESIGN_KEYS = frozenset({"controlPrompt"})
ALLOWED_REFERENCE_KEYS = frozenset({"audioPath", "promptText", "styleControl"})
ALLOWED_EXECUTION_KEYS = frozenset(
    {
        "delivery",
        "seed",
        "guidanceScale",
        "maxOutputTokens",
        "maxContextTokens",
        "temperature",
        "topK",
        "topP",
        "doSample",
        "repetitionPenalty",
        "depthDecoder",
        "collectTiming",
        "runtimeProfile",
    }
)
ALLOWED_DEPTH_DECODER_KEYS = frozenset({"doSample", "temperature", "topK", "topP"})
ALLOWED_RUNTIME_OPTION_KEYS = frozenset(
    {
        "fastAll",
        "fastTextEncoder",
        "fastBackbonePrefill",
        "fastBackboneDecode",
        "fastDepthDecoder",
        "fastCodec",
        "debugTokenHash",
    }
)
ALLOWED_MODES = frozenset({"plain", "voice_design", "reference_clone"})
ALLOWED_DELIVERIES = frozenset({"complete", "stream"})

FORBIDDEN_TOP_KEYS = frozenset(
    {
        "inference",
        "voice",
        "cfgValue",
        "inferenceTimesteps",
        "denoiseReference",
        "durationFactor",
        "lang",
        "emotion",
        "speaker",
        "segmentation",
        "text",
    }
)

DEFAULT_SEED = 42
DEFAULT_CFG_SCALE = 1.0
DEFAULT_MAX_NEW_TOKENS = 1500
DEFAULT_MAX_SEQ_LEN = 2048
DEFAULT_TEMPERATURE = 0.9
DEFAULT_TOP_K = 50
DEFAULT_TOP_P = 1.0
DEFAULT_DO_SAMPLE = True
DEFAULT_REPETITION_PENALTY = 1.1
DEFAULT_DEPTH_TEMPERATURE = 0.9
DEFAULT_DEPTH_TOP_K = 50
DEFAULT_DEPTH_TOP_P = 1.0
DEFAULT_DEPTH_DO_SAMPLE = True
SEED_MAX = 4294967295
MAX_NEW_TOKENS_CAP = 1500
MAX_SEQ_LEN_CAP = 2048


class RequestMapError(ValueError):
    """Structural / capability rejection before any model call."""


def _reject_unknown(obj: Mapping[str, Any], allowed: frozenset[str], label: str) -> None:
    extra = sorted(k for k in obj.keys() if k not in allowed)
    if extra:
        raise RequestMapError(f"{label} has unsupported keys: {', '.join(extra)}")


def _require_mapping(value: Any, label: str) -> Mapping[str, Any]:
    if not isinstance(value, Mapping):
        raise RequestMapError(f"{label} must be an object")
    return value


def _as_nonempty_str(value: Any, label: str) -> str:
    if not isinstance(value, str):
        raise RequestMapError(f"{label} must be a string")
    if value.strip() == "":
        raise RequestMapError(f"{label} must be non-empty")
    return value


def _as_optional_str(value: Any, label: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise RequestMapError(f"{label} must be a string or null")
    return value


def _as_int(value: Any, label: str, *, min_v: int, max_v: int) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise RequestMapError(f"{label} must be an integer")
    if value < min_v or value > max_v:
        raise RequestMapError(f"{label} must be in [{min_v}, {max_v}]")
    return value


def _as_finite_float(
    value: Any, label: str, *, gt: Optional[float] = None, gt0: bool = False
) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise RequestMapError(f"{label} must be a finite number")
    num = float(value)
    if not math.isfinite(num):
        raise RequestMapError(f"{label} must be finite")
    if gt0 and not (num > 0):
        raise RequestMapError(f"{label} must be greater than 0")
    if gt is not None and not (num > gt):
        raise RequestMapError(f"{label} must be greater than {gt}")
    return num


def _as_bool(value: Any, label: str) -> bool:
    if not isinstance(value, bool):
        raise RequestMapError(f"{label} must be a boolean")
    return value


def _as_top_p(value: Any, label: str) -> float:
    num = _as_finite_float(value, label, gt0=True)
    if num > 1.0:
        raise RequestMapError(f"{label} must be in (0, 1]")
    return num


def _join_direction(style: Optional[str], emotion_text: Optional[str]) -> Optional[str]:
    parts: list[str] = []
    if style is not None and style.strip() != "":
        parts.append(style)
    if emotion_text is not None and emotion_text.strip() != "":
        parts.append(emotion_text)
    if not parts:
        return None
    return "，".join(parts)


def parse_runtime_options(raw: Any) -> dict[str, Any]:
    if raw is None:
        return {
            "fastAll": None,
            "fastTextEncoder": False,
            "fastBackbonePrefill": False,
            "fastBackboneDecode": False,
            "fastDepthDecoder": False,
            "fastCodec": False,
            "debugTokenHash": False,
        }
    obj = _require_mapping(raw, "runtimeOptions")
    _reject_unknown(obj, ALLOWED_RUNTIME_OPTION_KEYS, "runtimeOptions")
    fast_all = obj.get("fastAll", None)
    if fast_all is not None and not isinstance(fast_all, bool):
        raise RequestMapError("runtimeOptions.fastAll must be boolean or null")
    return {
        "fastAll": fast_all,
        "fastTextEncoder": _as_bool(
            obj.get("fastTextEncoder", False), "runtimeOptions.fastTextEncoder"
        ),
        "fastBackbonePrefill": _as_bool(
            obj.get("fastBackbonePrefill", False), "runtimeOptions.fastBackbonePrefill"
        ),
        "fastBackboneDecode": _as_bool(
            obj.get("fastBackboneDecode", False), "runtimeOptions.fastBackboneDecode"
        ),
        "fastDepthDecoder": _as_bool(
            obj.get("fastDepthDecoder", False), "runtimeOptions.fastDepthDecoder"
        ),
        "fastCodec": _as_bool(obj.get("fastCodec", False), "runtimeOptions.fastCodec"),
        "debugTokenHash": _as_bool(
            obj.get("debugTokenHash", False), "runtimeOptions.debugTokenHash"
        ),
    }


def effective_fast_flags(options: Mapping[str, Any]) -> dict[str, bool]:
    master = options.get("fastAll", None)
    stages = {
        "fastTextEncoder": bool(options.get("fastTextEncoder", False)),
        "fastBackbonePrefill": bool(options.get("fastBackbonePrefill", False)),
        "fastBackboneDecode": bool(options.get("fastBackboneDecode", False)),
        "fastDepthDecoder": bool(options.get("fastDepthDecoder", False)),
        "fastCodec": bool(options.get("fastCodec", False)),
    }
    if master is True:
        return {k: True for k in stages}
    if master is False:
        return {k: False for k in stages}
    return stages


def parse_execution(raw: Any) -> dict[str, Any]:
    if raw is None:
        raw = {}
    obj = _require_mapping(raw, "execution")
    _reject_unknown(obj, ALLOWED_EXECUTION_KEYS, "execution")

    delivery = obj.get("delivery", "complete")
    if delivery not in ALLOWED_DELIVERIES:
        raise RequestMapError("execution.delivery must be complete|stream")

    seed_i = _as_int(obj.get("seed", DEFAULT_SEED), "execution.seed", min_v=0, max_v=SEED_MAX)
    cfg_f = _as_finite_float(
        obj.get("guidanceScale", DEFAULT_CFG_SCALE), "execution.guidanceScale", gt0=True
    )
    max_out_i = _as_int(
        obj.get("maxOutputTokens", DEFAULT_MAX_NEW_TOKENS),
        "execution.maxOutputTokens",
        min_v=1,
        max_v=MAX_NEW_TOKENS_CAP,
    )
    max_ctx_i = _as_int(
        obj.get("maxContextTokens", DEFAULT_MAX_SEQ_LEN),
        "execution.maxContextTokens",
        min_v=1,
        max_v=MAX_SEQ_LEN_CAP,
    )
    temperature_f = _as_finite_float(
        obj.get("temperature", DEFAULT_TEMPERATURE), "execution.temperature", gt0=True
    )
    top_k_i = _as_int(
        obj.get("topK", DEFAULT_TOP_K), "execution.topK", min_v=0, max_v=2_147_483_647
    )
    top_p_f = _as_top_p(obj.get("topP", DEFAULT_TOP_P), "execution.topP")
    do_sample_b = _as_bool(obj.get("doSample", DEFAULT_DO_SAMPLE), "execution.doSample")
    rep_f = _as_finite_float(
        obj.get("repetitionPenalty", DEFAULT_REPETITION_PENALTY),
        "execution.repetitionPenalty",
        gt0=True,
    )
    collect_timing_b = _as_bool(
        obj.get("collectTiming", False), "execution.collectTiming"
    )

    depth_raw = obj.get("depthDecoder")
    if depth_raw is None:
        depth = {
            "doSample": DEFAULT_DEPTH_DO_SAMPLE,
            "temperature": DEFAULT_DEPTH_TEMPERATURE,
            "topK": DEFAULT_DEPTH_TOP_K,
            "topP": DEFAULT_DEPTH_TOP_P,
        }
    else:
        depth_obj = _require_mapping(depth_raw, "execution.depthDecoder")
        _reject_unknown(depth_obj, ALLOWED_DEPTH_DECODER_KEYS, "execution.depthDecoder")
        depth = {
            "doSample": _as_bool(
                depth_obj.get("doSample", DEFAULT_DEPTH_DO_SAMPLE),
                "execution.depthDecoder.doSample",
            ),
            "temperature": _as_finite_float(
                depth_obj.get("temperature", DEFAULT_DEPTH_TEMPERATURE),
                "execution.depthDecoder.temperature",
                gt0=True,
            ),
            "topK": _as_int(
                depth_obj.get("topK", DEFAULT_DEPTH_TOP_K),
                "execution.depthDecoder.topK",
                min_v=0,
                max_v=2_147_483_647,
            ),
            "topP": _as_top_p(
                depth_obj.get("topP", DEFAULT_DEPTH_TOP_P),
                "execution.depthDecoder.topP",
            ),
        }

    runtime_profile = obj.get("runtimeProfile")
    profile = parse_runtime_options(runtime_profile) if runtime_profile is not None else None

    return {
        "delivery": delivery,
        "seed": seed_i,
        "guidanceScale": cfg_f,
        "maxOutputTokens": max_out_i,
        "maxContextTokens": max_ctx_i,
        "temperature": temperature_f,
        "topK": top_k_i,
        "topP": top_p_f,
        "doSample": do_sample_b,
        "repetitionPenalty": rep_f,
        "depthDecoder": depth,
        "collectTiming": collect_timing_b,
        "runtimeProfile": profile,
    }


def map_speech_request(body: Mapping[str, Any]) -> dict[str, Any]:
    if not isinstance(body, Mapping):
        raise RequestMapError("request body must be an object")

    for bad in FORBIDDEN_TOP_KEYS:
        if bad in body:
            raise RequestMapError(f"unsupported field for Breeze: {bad}")
    _reject_unknown(body, ALLOWED_TOP_KEYS, "request")

    model = body.get("model")
    if not isinstance(model, str) or model.strip() == "":
        raise RequestMapError("model must be a non-empty string")

    mode = body.get("mode", "plain")
    if mode not in ALLOWED_MODES:
        raise RequestMapError("mode must be plain|voice_design|reference_clone")

    text = _as_nonempty_str(body.get("input"), "input")

    voice_design = body.get("voiceDesign")
    reference = body.get("reference")
    execution = parse_execution(body.get("execution"))
    runtime_options = parse_runtime_options(body.get("runtimeOptions"))
    if execution["runtimeProfile"] is not None:
        runtime_options = execution["runtimeProfile"]

    request_id = body.get("requestId")
    if request_id is not None:
        if not isinstance(request_id, str) or request_id.strip() == "":
            raise RequestMapError("requestId must be a non-empty string when provided")

    instruction: Optional[str] = None
    ref_audio_path: Optional[str] = None
    ref_text: Optional[str] = None

    if mode == "plain":
        if voice_design is not None:
            raise RequestMapError("plain mode must not carry voiceDesign")
        if reference is not None:
            raise RequestMapError("plain mode must not carry reference")
        template_name = "tts_plain"

    elif mode == "voice_design":
        if reference is not None:
            raise RequestMapError("voice_design mode must not carry reference")
        if voice_design is None:
            raise RequestMapError("voice_design mode requires voiceDesign.controlPrompt")
        vd = _require_mapping(voice_design, "voiceDesign")
        _reject_unknown(vd, ALLOWED_VOICE_DESIGN_KEYS, "voiceDesign")
        instruction = _as_nonempty_str(vd.get("controlPrompt"), "voiceDesign.controlPrompt")
        template_name = "tts_instruction"

    else:
        if voice_design is not None:
            raise RequestMapError("reference_clone mode must not carry voiceDesign")
        if reference is None:
            raise RequestMapError("reference_clone requires reference")
        ref = _require_mapping(reference, "reference")
        _reject_unknown(ref, ALLOWED_REFERENCE_KEYS, "reference")
        ref_audio_path = _as_nonempty_str(ref.get("audioPath"), "reference.audioPath")
        ref_text = _as_nonempty_str(ref.get("promptText"), "reference.promptText")
        style = _as_optional_str(ref.get("styleControl"), "reference.styleControl")
        instruction = _join_direction(style, None)
        template_name = "ref_edit_tata" if instruction else "ref_clone_tata"

    if execution["guidanceScale"] != 1.0 and template_name in ("tts_plain", "ref_clone_tata"):
        raise RequestMapError(
            f"guidanceScale!=1 is unsupported for template {template_name} "
            "(no negative prompt branch)"
        )

    upstream: dict[str, Any] = {
        "id": request_id or "single-request",
        "text": text,
        "speaker": "S0",
    }
    if instruction is not None:
        upstream["instruction"] = instruction
    if ref_audio_path is not None:
        upstream["ref_audio_path"] = ref_audio_path
        upstream["ref_text"] = ref_text

    audit = {
        "mode": mode,
        "templateName": template_name,
        "text": text,
        "instruction": instruction,
        "refAudioPath": ref_audio_path,
        "refText": ref_text,
        "execution": execution,
        "runtimeOptions": runtime_options,
        "effectiveFast": effective_fast_flags(runtime_options),
    }

    return {
        "model": model,
        "template_name": template_name,
        "upstream_request": upstream,
        "execution": execution,
        "runtime_options": runtime_options,
        "request_id": request_id,
        "audit": audit,
    }


def map_direction_controls(
    *,
    style_control: Optional[str],
    emotion_text: Optional[str],
) -> Optional[str]:
    return _join_direction(style_control, emotion_text)


def apply_direction_to_body(
    body: MutableMapping[str, Any],
    *,
    style_control: Optional[str] = None,
    emotion_text: Optional[str] = None,
) -> Mapping[str, Any]:
    merged = dict(body)
    ref = dict(_require_mapping(merged.get("reference") or {}, "reference"))
    joined = _join_direction(
        style_control if style_control is not None else ref.get("styleControl"),
        emotion_text,
    )
    if joined is not None:
        ref["styleControl"] = joined
    elif "styleControl" in ref and (ref.get("styleControl") or "").strip() == "":
        ref.pop("styleControl", None)
    merged["reference"] = ref
    return map_speech_request(merged)
