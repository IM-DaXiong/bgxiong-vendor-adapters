# HOST_MAPPING · RunningHub MiniMax H3 r2v turbo (V2)

Local author source (sibling pack, may differ by machine):

`../bgxiong-ComfyUI/API/minimax-h3/video_minimax_h3_r2v_turbo.json`

Runtime: RunningHub cloud `POST /openapi/v2/run/workflow/2100758868111486978`.
This crate does not `POST /prompt` to LAN Comfy.

## Bindings

| RCD / HOST_PAYLOAD | Node | field |
|---|---|---|
| `prompt` | 138 PrimitiveStringMultiline | `value` |
| `durationSeconds` | 132 PrimitiveFloat | `value` |
| `fps` | 130 CreateVideo | `fps` |
| fps expression | 131 ComfyMathExpression | `expression` |
| `aspect` | 115 ResolutionSelector | `aspect_ratio` |
| `resolution` | 115 ResolutionSelector | `megapixels` |
| refs 1..=2 | upload → 137 / 139 LoadImage `image` | `referenceImagesB64` |
| RCD modes | `implementedModeIds = ["multi_image_to_video"]` | not I2V / first_last_frame |

Workbench intent is **multi-ref** (same as LAN H3 r2v turbo). Host must not
invent `image_to_video` / `first_last_frame` from `maxReferenceImages=2`.

## Do not copy from LAN pin

LAN `comfyui-lan-minimax-h3` pin `h3.r2v.turbo.api.json` uses LoadImage `141`
and LoRA `142`. **This uploaded UI graph** uses:

- `141` MiniMaxH3TurboLoRA (leave on graph; never `nodeInfoList` image)
- `142` MiniMaxH3TurboSampler (leave on graph)

`startFrameB64` / `endFrameB64` with empty refs fold into 137/139 (1..=2).
