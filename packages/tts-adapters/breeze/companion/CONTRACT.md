# Breeze TTS 2 Sidecar HTTP Contract (V1)

> Shared TtsSidecarManager / port pool. Independent process, venv, wheelhouse, checkpoints.
> Implementation: `sidecar_server.py` · mapping: `request_map.py` · runtime: `breeze_runtime.py`
> Upstream freeze: `breezeblue-ai/breeze-tts@6eba901d` · model `BreezeBlue/Breeze-TTS-2@3e28c515`

**Bind**: `127.0.0.1` only · port from `BGX_TTS_SIDECAR_PORT` (manager-assigned)

**Windows x64 + managed CPython 3.12 + CUDA eager** is the certified target for this round.
`start-sidecar.sh` is provided for parity; other platforms are not claimed verified.

---

## GET `/health`

**200** when model loaded:

```json
{
  "status": "ok",
  "engine": "breeze",
  "modelLoaded": true,
  "version": "1.0.0",
  "sampleRate": 24000,
  "profileFingerprint": "<sha256>",
  "requestedProfile": {},
  "effectiveProfile": {}
}
```

While loading: `"status": "loading"`, `"modelLoaded": false`.

---

## GET `/v1/models`

Manager model list (OpenAI-shaped). Loaded checkpoint identity is local config id on the host;
sidecar reports frozen default `BreezeBlue/Breeze-TTS-2`.

## GET `/v1/model`

F10 metadata: `frame_rate`, `model_digest`, `max_new_tokens`, `max_batch_texts`, plus
`sampleRate`, profile fingerprint, effective options.

---

## POST `/v1/audio/speech`

**Content-Type**: `application/json`

Unknown keys / modes → **400**. Model must match loaded checkpoint (no on-the-fly download).

### Modes

| mode | Required | Upstream template |
|------|----------|-------------------|
| `plain` | `input` | `tts_plain` |
| `voice_design` | `voiceDesign.controlPrompt` | `tts_instruction` |
| `reference_clone` | `reference.audioPath` + `reference.promptText` | `ref_clone_tata` or `ref_edit_tata` if `styleControl` |

`reference.promptText` is the accurate full transcript of the reference audio (required; no ASR fill-in).
Voice Direction (F04) uses `reference_clone` + `styleControl` (host may join style + emotion text).

### Execution (optional)

| Field | Default | Notes |
|-------|---------|-------|
| delivery | complete | `complete` → `audio/wav`; `stream` → `audio/pcm` s16le |
| seed | 42 | int 0..4294967295 |
| guidanceScale | 1 | >0; !=1 rejected for plain/clone (no negative branch) |
| maxOutputTokens | 1500 | 1..1500 |
| maxContextTokens | 2048 | 1..2048 |
| temperature / topK / topP / doSample | 0.9 / 50 / 1.0 / true | |
| repetitionPenalty | 1.1 | |
| depthDecoder.* | 0.9 / 50 / 1.0 / true | maps depth_decoder generation_config |
| collectTiming | false | |

### Responses

- **200 complete**: `audio/wav`, mono PCM16, sample rate from runtime (24000)
- **200 stream**: `audio/pcm`, headers `X-Sample-Rate`, `X-Sample-Format=s16le`, `X-Request-Id`
- **400**: structural / capability rejection
- **409**: busy (single-flight lock)
- **503**: model not ready
- **5xx**: inference failure

Host must verify `X-Request-Id` and `GET /v1/requests/{requestId}` after stream EOF; HTTP 200/EOF alone is not success.

---

## POST `/v1/audio/speech/batch`

Native eager batch (not looped singles). Shared mode/reference/execution; `items:[{id,input}]`.
Body: concatenated s16le PCM; `X-Segment-Bytes`, `X-Segment-Ids`, `X-Sample-Rate`, `X-Sample-Format`.

---

## GET `/v1/requests/{requestId}`

`running|complete|failed|cancelled|unknown`. Reports sampleCount, pcmBytes, digest, error.
Lifecycle-bound to this sidecar process; cleaned status → unknown (not inferred complete).

---

## Runtime options / profile

Env `BGX_BREEZE_RUNTIME_OPTIONS` JSON (optional) at start:
`fastAll` (null|true|false), five stage flags, `debugTokenHash`.
Master non-null overrides stages. Busy profile change is rejected by manager (host).

---

## Forbidden

- VoxCPM `inference` / `cfgValue` / `denoiseReference`
- IndexTTS `emotion` / `durationFactor` / `lang` / `speaker`
- Silent CFG for plain/clone by fabricating negative instructions
- WSL / Docker / CPU-only counted as Windows CUDA S0 pass


---

## POST `/v1/audio/reference-temp`

Companion-scoped temporary reference upload (SDK media bridge). **Not** a public vendor API.

- **Body**: `multipart/form-data` with file field `file` (audio bytes authorized by the host).
- **200 JSON**: `{ tempId, audioPath, fileName, originalFileName, mime, byteLength, expiresAtUnix }`
  - `audioPath` is only valid inside this sidecar; adapters must not invent host absolute paths.
- **400**: empty/too small/too large/unsupported MIME/invalid name
- Lease root: `{runtime}/tmp_refs/`; expired leases cleaned opportunistically.
- Default TTL: `BGX_BREEZE_TMP_REF_TTL_SEC` (default 3600).

## DELETE `/v1/audio/reference-temp/{tempId}`

Releases one temp lease early. Missing id returns `{deleted:false}`.

After upload, `POST /v1/audio/speech` with `mode=reference_clone` uses the returned `audioPath` plus required `reference.promptText` (and optional `styleControl` for direction).
