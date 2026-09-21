# HOST_MAPPING · RunningHub MiniMax H3 r2v 1/2/3 slots (OpenAPI V2 AI App)

Runtime: `POST /openapi/v2/run/ai-app/{APP_ID}` per `modelId`.
Query: `/openapi/v2/query`. Upload: `/media/upload/binary`.
This crate does not `POST /prompt` to LAN Comfy and does not use
`/run/workflow` or legacy `/task/openapi/ai-app/run`.

| modelId | APP_ID | LoadImage |
|---|---|---|
| `h3.r2v.1slot` | `2101954317581381633` | 114 |
| `h3.r2v.2slot` | `2101956370017906690` | 137 / 139 (141 is TurboLoRA) |
| `h3.r2v.3slot` | `2101957517390737410` | 137 / 139 / 143 (141 is TurboLoRA) |

## Widgets (not shared)

### 1-slot

| RCD / HOST_PAYLOAD | Node | field |
|---|---|---|
| `prompt` | 132 | `prompt` |
| `durationSeconds` | 134 | `value` |
| `fps` | 131 + 133 | `fps` / `expression` |
| `aspect` / `resolution` | 115 | `aspect_ratio` / `megapixels` |

### 2-slot / 3-slot

| RCD / HOST_PAYLOAD | Node | field |
|---|---|---|
| `prompt` | 138 | `value` |
| `durationSeconds` | 132 | `value` |
| `fps` | 130 + 131 | `fps` / `expression` |
| `aspect` / `resolution` | 115 | `aspect_ratio` / `megapixels` |

RCD `implementedModeIds = ["multi_image_to_video"]` on every slot.
Feature `video-reference-media-v1`. Empty `referenceImages` folds
`startFrame` / `endFrame` handles, up to maxRefs. Inline Base64 is forbidden.
