# HOST_MAPPING · RunningHub MiniMax H3 r2v + fl2va + soundtrack (OpenAPI V2 AI App)

All slots: `POST /openapi/v2/run/ai-app/{APP_ID}`.
Query: `/openapi/v2/query`. Upload: `/media/upload/binary`.
This crate does not `POST /prompt` to LAN Comfy and does not use the
legacy `/task/openapi/ai-app/run` envelope.

| modelId | endpoint | LoadImage |
|---|---|---|
| `h3.r2v.1slot` | AI App `2108667855574224898` | 137 (never 114 / 139 / 141) |
| `h3.i2v.turbo` | AI App `2101954317581381633` | 114 |
| `h3.r2v.2slot` | AI App `2101956370017906690` | 137 / 139 (141 is TurboLoRA) |
| `h3.r2v.3slot` | AI App `2101957517390737410` | 137 / 139 / 143 (141 is TurboLoRA) |
| `h3.r2v.4slot` | AI App `2103015554197053441` | 137 / 139 / 143 / 144 (141 is TurboLoRA) |
| `h3.fl2va.s20` | AI App `2107978021675294722` | 114 |
| `h3.soundtrack.1ref` | AI App `2107982892470198274` | 137 (142 is Load Audio) |

## Widgets (not shared)

### 1-slot (`h3.r2v.1slot`)

| SDK v2 input | Node | field |
|---|---|---|
| `prompt` | 138 | `value` |
| `durationSeconds` | 132 | `value` |
| image | 137 | `image` |
| `aspect` / `resolution` | 115 | `aspect_ratio` / `megapixels` |
| closed pin | 136 | `ref_image_size` = `match` |

Native fps is fixed at 24; no adjustable fps node. Do not write nodes `130`, `131`, or `129` `noise_seed`. CLIP node `128` is `clip_name` only.

### i2v turbo

| SDK v2 input | Node | field |
|---|---|---|
| `prompt` | 132 | `prompt` |
| `durationSeconds` | 134 | `value` |
| `fps` | 131 + 133 | `fps` / `expression` |
| `aspect` / `resolution` | 115 | `aspect_ratio` / `megapixels` |

### 2-slot / 3-slot / 4-slot

| SDK v2 input | Node | field |
|---|---|---|
| `prompt` | 138 | `value` |
| `durationSeconds` | 132 | `value` |
| `fps` | 130 + 131 | `fps` / `expression` |
| `aspect` / `resolution` | 115 | `aspect_ratio` / `megapixels` |

### fl2va (`h3.fl2va.s20`)

| SDK v2 input | Node | field |
|---|---|---|
| `prompt` | 133 | `prompt` |
| `durationSeconds` | 135 | `value` |
| `aspect` / `resolution` | 115 | `aspect_ratio` / `megapixels` |
| image | 114 | `image` |

Native fps is fixed at 24; no adjustable fps node. Node `131` on this App is `noise_seed` and must not be written.

### soundtrack (`h3.soundtrack.1ref`)

| SDK v2 input | Node | field |
|---|---|---|
| `prompt` | 138 | `value` |
| `durationSeconds` | 132 | `value` |
| image | 137 | `image` |
| `referenceAudios` | 142 | `audio` |
| `aspect` / `resolution` | 115 | `aspect_ratio` / `megapixels` |
| closed pin | 136 | `ref_image_size` = `match` |

Native fps is fixed at 24; no adjustable fps node. Steps are `20` on node `124`. Do not reuse the 2-slot steps-`6` table.
Zero images or zero `referenceAudios` is a submit error.

SDK v2 inputMode is multi_image_to_video on every declared operation.
SDK v2 mediaSlots carry exact ordered mediaId references and leased originals.
Each workflow requires exactly its declared image count; soundtrack requires one audio.
No legacy startFrame/endFrame folding, inline Base64, or implicit stored input reuse.

## Weight pins (every submit)

OpenAPI `nodeInfoList` is a merge overlay. CLIP / UNet / VAE / scheduler must
be sent every time. Do not copy i2v-turbo node ids onto ref2va graphs.

### 1ref

| Node | fields |
|---|---|
| 128 | `clip_name` only (no type / device) |
| 127 | `unet_name` ref2va, `weight_dtype` `default` |
| 119 / 120 | video / audio `vae_name` |
| 124 | `steps` `6`, `scheduler` `simple`, `denoise` `1` |
| 136 | `ref_image_size` `match` |

### i2v turbo

| Node | fields |
|---|---|
| 129 | `clip_name` `qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors`, `type` `minimax`, `device` `default` |
| 128 | `unet_name` `minimax_h3_fl2va_pruned_int8_convrot.safetensors`, `weight_dtype` `default` |
| 121 / 122 | video / audio `vae_name` |
| 125 | `steps` `6`, `scheduler` `simple`, `denoise` `1` |

### 2-slot / 3-slot / 4-slot

| Node | fields |
|---|---|
| 128 | CLIP `clip_name` plus `type` `minimax` and `device` `default` |
| 127 | `unet_name` `minimax_h3_ref2va_pruned_int8_convrot.safetensors` (not fl2va) |
| 119 / 120 | video / audio `vae_name` |
| 124 | `steps` `6`, `scheduler` `simple`, `denoise` `1` |

### fl2va

| Node | fields |
|---|---|
| 130 | `clip_name` only (no type / device) |
| 129 | `unet_name` fl2va, `weight_dtype` `default` |
| 121 / 122 | video / audio `vae_name` |
| 126 | `steps` `20`, `scheduler` `simple`, `denoise` `1` |

### soundtrack

| Node | fields |
|---|---|
| 128 | `clip_name` only |
| 127 | `unet_name` ref2va, `weight_dtype` `default` |
| 119 / 120 | video / audio `vae_name` |
| 124 | `steps` `20`, `scheduler` `simple`, `denoise` `1` |
| 136 | `ref_image_size` `match` |

Upload `fieldValue` for LoadImage: `data.fileName` unchanged (`openapi/<hash>.ext`). Do not use `download_url` and do not strip `openapi/`.

## 1.0.2 工作台合同修复

`resolution` 使用 `mp:0.25`、`mp:0.4`、`mp:0.6`、`mp:1` 四个命名档，包内转换到原有 megapixels 节点；不声明连续范围。七个操作不声明 seed，宿主不得注入 seed 或未声明的 advanced，原云端种子策略不变。仅 soundtrack 声明 `advanced.audioUsage=soundtrack_copy`，表示完整原音驱动对口型，提交前校验，不是附加云端节点。其余六项不造音频用途。
