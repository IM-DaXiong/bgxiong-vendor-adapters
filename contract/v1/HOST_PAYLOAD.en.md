# Host ↔ Guest Field Contract (Vendor Adapter Protocol v1)

> **Project external developer standard (human-readable field dictionary)**  
> Third-party adapter authors **must** read this before implementing a plugin.  
> Wire field names are always **camelCase**.  
> Closed sets (operations, capabilitySlots, errorCodes, …) live in `protocol/vendor-adapter-protocol.json` (the Rust SDK embeds that file next to `lib.rs`). Host trust paths and providerKinds stay in `packages/app-contracts/v1/vendor-adapter.json`.  
> 中文版: [`HOST_PAYLOAD.zh-CN.md`](HOST_PAYLOAD.zh-CN.md)

## Authority map

| Role | Path |
|------|------|
| **This document (English field dictionary)** | `packages/vendor-adapter-contract/v1/HOST_PAYLOAD.en.md` |
| Chinese edition | `HOST_PAYLOAD.zh-CN.md` |
| WIT world + host imports | `wit/vendor-adapter.wit` |
| Envelope & response helpers | `sdk/rust` (`Invocation`, `Response`, `SubmitResult`, `QueryResult`, `Output`) |
| Manifest & runtime-caps schemas | `schemas/manifest.schema.json`, `schemas/runtime-caps.schema.json` |
| Host implementation cross-check | `src-tauri/src/vendor_adapter_gateway/bridge/*_provider.rs` |
| Capability projection | `src-tauri/src/vendor_adapter_gateway/bridge/caps_adapter.rs` |
| RCD validation | `src-tauri/src/vendor_adapter_gateway/domain/runtime_caps.rs` |

`examples/` are **specimens only**, not the field standard. Standard = this document (or the Chinese edition) + WIT + schemas + SDK types.

---

## 0. Design rules

1. Whatever the user selected in the UI is what must run. No silent vendor/channel fallback.
2. The host never reads vendor workflow JSON. Adjustable UI params must be declared in RCD and actually applied (return `appliedParams`).
3. Prefer `host-media` handles for large binaries; do not assume huge Base64 forever.
4. Guest must not import WASI sockets / filesystem / cli. Only WIT `host-*` imports.
5. Exactly one of `dataJson` / `error` per response.
6. Async `vendorTaskId` must be the real vendor id (polling + resume depend on it). Never invent one.
7. User-visible toasts are mapped by the host from `error.code`. Do not put serde/stack traces in `error.message` for end users.

---

## 1. Call model

Each host call invokes the guest export `invoke` with an **Invocation envelope**.  
Business fields live in **`payloadJson`**: a **string** whose content is a JSON object.

```
Host                                      Guest
----                                      -----
Invocation {
  protocolVersion,
  operation,           // capabilities|probe|upload|submit|query|cancel
  profileId,
  capabilitySlot,      // image|video|text|...
  requestId,
  budgetMs,
  payloadJson,         // business JSON, stringified
  bindingId,
  negotiatedVersion?,
  enabledFeatures?[]
}
                 -- invoke -->
                                          Response {
                                            protocolVersion,
                                            operation,
                                            dataJson?  XOR  error?
                                          }
```

Outbound HTTP/media/credentials go only through WIT imports: `host-http`, `host-media`, `host-credential`, `host-state`, `host-log`.

---

## 2. Invocation envelope (Host → Guest)

| Wire field | Type | Meaning |
|------------|------|---------|
| `protocolVersion` | `u32` | Protocol major. Echo host major in every response (`protocol_host_major()` in SDK). |
| `operation` | `string` | Operation name; closed set `operations` in the contract JSON. |
| `profileId` | `string` | Host-side profile / binding id. |
| `capabilitySlot` | `string` | Capability slot; closed set `capabilitySlots`. |
| `requestId` | `string` | Correlation id for this invoke (often tied to an AI task). |
| `budgetMs` | `u64` | Remaining execution budget in milliseconds (monotonic remaining, not a wall-clock deadline). |
| `payloadJson` | `string` | **Stringified** business JSON; interpret by `operation` × `capabilitySlot` (section 3). |
| `bindingId` | `string` | Host binding id for this invoke (not `ai_tasks.id`; that stays host-private). |
| `negotiatedVersion` | `u32?` | Negotiated protocol version (optional). |
| `enabledFeatures` | `string[]?` | Enabled optional features (optional; default empty). |

SDK type: `Invocation`. Deserialize business fields with `inv.payload::<T>()`.

---

## 3. `payloadJson` business fields (Host → Guest)

Fields below are what the **current desktop host bridge actually sends**.  
Do not rely on unlisted keys; ignore unknowns for forward compatibility.  
Missing key or JSON `null` means “not provided”.

### 3.1 Generic operations

#### `capabilities`

Usually `{}`. Ask the guest to return a runtime-caps document conforming to `schemas/runtime-caps.schema.json` (section 5).

#### `probe`

Usually `{}`. Liveness / connectivity probe.

**Import L3** (empty payload, no credential) is not a vendor liveness check: the guest must succeed and **must not** call `host-http`. Outbound probe is allowed only after the host injects a non-empty top-level **`endpointBaseUrl`**.

When the manifest declares `userConfigurable`, the host copies the adapter credential `baseUrl` into top-level **`endpointBaseUrl`** (submit/query/probe via `invoke_adapter_operation`, and boot-resume query). This is a generic field, not a vendor-specific API. Import and the settings "Probe" button do not inject that field.

#### `upload`

Depends on slot and media pipeline. Prefer `host-media` handles over embedding large bytes in `payloadJson`.

#### `cancel`

| Wire field | Type | Meaning |
|------------|------|---------|
| `vendorTaskId` | `string` | Vendor task id to cancel (if the slot implements cancel). |

If cancel is unsupported, return a contract error code (e.g. `adapterCapabilityDenied`); do not trap.

---

### 3.2 `submit` / `query` by capability slot

#### Slot `video` (and `keyframe` routed through the video pipeline)

`keyframe` shares the **same** submit/query shape as `video`. Distinguish via envelope `capabilitySlot`.

**submit**

| Wire field | Type | Meaning |
|------------|------|---------|
| `prompt` | `string` | User prompt. |
| `model` | `string` | Vendor model / workflow id — the part **after** `adapter:{pluginId}:`. |
| `negativePrompt` | `string?` | Negative prompt. |
| `resolution` | `string?` | Resolution / frame size (e.g. `1920x1080`). Aligns with RCD `resolution` / `aspect`. |
| `fps` | `number?` | Frames per second. |
| `durationSeconds` | `number?` | Duration in seconds. |
| `outputFormat` | `string?` | Desired output container/format. |
| `startFrameB64` | `string?` | Start frame as Base64 (may be large; production prefers media handles). Guest uploads must sniff magic for MIME/filename; unknown bytes fail. |
| `endFrameB64` | `string?` | End frame as Base64 (same rule; never default to PNG). |
| `referenceImagesB64` | `string[]?` | Reference images as Base64 list. |
| `extra` | `object?` | Host extra context. **Not** a stable vendor API; only read documented keys if any. |
| `endpointBaseUrl` | `string?` | Credential `baseUrl` (`userConfigurable`). Guest builds vendor HTTP from it. |

**query**

| Wire field | Type | Meaning |
|------------|------|---------|
| `vendorTaskId` | `string` | Vendor task id from submit. |
| `model` | `string?` | Same model id as submit. |
| `endpointBaseUrl` | `string?` | Same as submit: credential origin. |

---

#### Slots `image` / `imageToImage`

Same field set. With references the slot is usually `imageToImage`; otherwise `image`.

**submit**

| Wire field | Type | Meaning |
|------------|------|---------|
| `prompt` | `string` | Prompt. |
| `model` | `string` | Vendor model id. |
| `size` | `string?` | Size (e.g. `1920x1088`). |
| `negativePrompt` | `string?` | Negative prompt. |
| `responseFormat` | `string?` | Vendor response format preference. |
| `watermark` | `bool?` | Watermark flag if applicable. |
| `referenceImagesB64` | `string[]?` | Reference images; typically non-empty for `imageToImage`. |
| `extra` | `object?` | Extra context. |

**Image size field map (one name, one meaning)**

| Layer | Field | Meaning |
|---|---|---|
| submit `payloadJson` | `size` | Workbench clarity, `WxH` |
| RCD slot param | `resolution` | `enum` presets; `options[].value` equals `size`. **Do not** declare a separate `aspect` param; aspect is option metadata `aspectRatio` (`16:9` / `9:16` / `1:1` / `4:3` / `3:4`) |
| submit response | `appliedParams.resolution` | The `WxH` actually written into the vendor request; must match the chosen `size` |

Image slots **must not** use `resolution.mode=range` (a 1-D scalar cannot drive width×height columns). For `fixed`, the host omits `size`; if the guest still receives `size`, return `adapterInvalidRequest`. Missing / unnegotiated RCD is treated as workflow-native size (no `size` on the wire).

**query**

| Wire field | Type | Meaning |
|------------|------|---------|
| `vendorTaskId` | `string` | Vendor task id. |
| `model` | `string?` | Model id. |

---

#### Slot `text`

**submit** is **await-style**: guest must return `kind: "completed"`. The host does **not** poll.

| Wire field | Type | Meaning |
|------------|------|---------|
| `model` | `string` | Vendor model id. |
| `messages` | `{role, content}[]` | Chat messages. |
| `temperature` | `number?` | Sampling temperature. |
| `responseFormatJson` | `bool?` | Whether JSON-shaped output is required. |
| `maxTokens` | `number?` | Max generation tokens. |
| `thinking` | `bool?` | Enable thinking / reasoning if supported. |

`query` is not used for text in the current host.

---

#### Slot `speech`

**submit**

| Wire field | Type | Meaning |
|------------|------|---------|
| `text` | `string` | Text to synthesize. |
| `voiceId` | `string?` | Voice id. |
| `model` | `string?` | Model id. |
| `mode` | `string?` | Synthesis mode (host enum Debug string). |

**query**: `{ "vendorTaskId": "…" }`

---

#### Slot `avatar`

**submit**

| Wire field | Type | Meaning |
|------------|------|---------|
| `imagePath` | `string` | Driving image path (host local path semantics). |
| `audioPath` | `string?` | Driving audio path. |
| `prompt` | `string?` | Extra prompt. |
| `extra` | `object?` | Extra context. |

**query**: `{ "vendorTaskId": "…" }`

---

#### Slot `lipsync`

**submit**

| Wire field | Type | Meaning |
|------------|------|---------|
| `audioPath` | `string` | Input audio path. |
| `inputMode` | `string?` | Input mode (enum Debug string). |
| `extra` | `object?` | Extra context. |

**query** / **cancel**: `{ "vendorTaskId": "…" }`

---

#### Slot `sfx`

**submit**

| Wire field | Type | Meaning |
|------------|------|---------|
| `prompt` | `string` | SFX description. |
| `aiTaskId` | `string?` | Host AI task id. |

---

#### Slots `voiceDesign` / `videoUpscale` / `videoFaceRefine` / `imageUpscale`

Reserved in the contract closed set. Implement only if the **same client version** has a bridge provider for that slot; otherwise do not advertise generate-ready support.

---

## 4. Response (Guest → Host)

Envelope: `protocolVersion` + `operation` + **exactly one** of `dataJson` or `error`.

### 4.1 `error`

| Wire field | Meaning |
|------------|---------|
| `code` | Must be one of contract `errorCodes`. |
| `message` | Diagnostic text for logs; host maps user copy from `code`. |
| `retryable` | Whether the host may retry. |
| `vendorCode` | Optional vendor error code. |
| `vendorRequestId` | Optional vendor request id. |

### 4.2 `submit` success (`dataJson` object)

Serialized from SDK `SubmitResult` (`serde` tag = `kind`).

#### `kind: "accepted"` (async)

| Field | Meaning |
|-------|---------|
| `vendorTaskId` | **Real** vendor task id; host polls and resumes with it. Never invent. |
| `appliedParams` | Optional; see 4.5. |

#### `kind: "completed"` (sync)

| Field | Meaning |
|-------|---------|
| `outputs` | Final `Output[]`. |
| `appliedParams` | Optional; see 4.5. |

### 4.3 `query` success

| Field | Meaning |
|-------|---------|
| `status` | One of `taskStatuses`: `queued` / `running` / `succeeded` / `failed` / `cancelled` / `expired`. |
| `outputs` | `Output[]` when finished (may be empty while running). |
| `progressText` | Optional progress text. |

Some host paths also read numeric `progress`; prefer contract fields first.

### 4.4 `Output`

| Field | Meaning |
|-------|---------|
| `mediaKind` | e.g. `image` / `video` / `audio`. |
| `source` | `url` \| `host-media` \| `inline-base64` \| `inline-binary`. |
| `value` | URL, host media handle, or Base64/etc. **Canonical key is `value`** (legacy `url`/`data` are not primary). |
| `mime` | Optional MIME type. |

For sync text completion, the host may also read text from `outputs[0].text` (or a compatibility key). New plugins should follow the versioned host expectation and this contract.

### 4.5 `appliedParams` (prove adjustable params were applied)

When RCD declares `range` / `enum` for `duration` / `fps` / `resolution` / `aspect` (with non-empty `bindingId`), a successful submit should include the values actually written to the vendor request:

| Field | Meaning |
|-------|---------|
| `duration` | Applied duration. |
| `fps` | Applied fps. |
| `resolution` | Applied resolution. |
| `aspect` | Applied aspect. |

Declaring a control but not applying it → **host hard failure**.  
SDK: `submit_accepted_with_applied` / `submit_completed_with_applied` + `AppliedParams`.

---

## 5. `capabilities` response (runtime caps)

`dataJson` must conform to `schemas/runtime-caps.schema.json`:

| Area | Requirement |
|------|-------------|
| Top-level | `schemaVersion` (1), `pluginId`, `pluginVersion`, `slots[]` |
| Each slot | Required `slot`, `modelId`; optional `duration` / `fps` / `resolution` / `aspect` / `maxReferenceImages` / `supportsFirstLastFrame` / `implementedModeIds` / `featureModules` |
| Param modes | `fixed` \| `range` \| `enum`; `range`/`enum` **require** non-empty `bindingId` |
| `enum` option | Required `id` / `label` / `value`; optional `aspectRatio` (closed five aspects). Image `resolution` enum **must** set `aspectRatio` |

The host only understands this document — never a vendor node graph.

---

## 6. How developers should use this

1. Before coding: **this file → WIT → schemas → `sdk/rust` types**.
2. With Cursor / Codex: attach **this contract + `wit/` + `sdk/rust` + the vendor’s official API/SDK**. Specimens are optional references only.
3. Integrate: Settings → Third-party adapters → import crate root → pick `adapter:{pluginId}:{model}` in the matching workbench. On failure, check `payloadJson` / response against this table.
4. After client upgrades, re-read **the same version** of this file and `bridge/*_provider.rs`. Release notes should call out changes here.

---

## 7. Non-goals

1. Not “any HTTP vendor / any slot works without host changes”.
2. `extra`, local paths, and huge Base64 may evolve; the stable contract is the envelope + named fields above + RCD + outputs.
3. L2 hello / probe green ≠ generate-ready.

---

## 8. Change control

1. Additive optional fields should be ignored by older guests.
2. Renames / meaning changes of existing wire fields require a protocol or documented major bump and host+SDK updates together.
3. Keep **both** language editions in sync when editing `bridge/*_provider.rs` or SDK response helpers.
