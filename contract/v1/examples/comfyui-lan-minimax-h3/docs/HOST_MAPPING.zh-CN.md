# HOST_MAPPING · LAN Comfy MiniMax H3

UI source (sibling pack, may differ by machine):
`../bgxiong-ComfyUI/MiniMax-H3/workflows/official/video_minimax_h3_{t2v,i2v,r2v}.json`

Runtime pins (this crate):

- `h3.t2v` → `workflows/h3.t2v.api.json` (`WORKFLOW_PIN`)
- `h3.i2v.turbo` → `workflows/h3.i2v.turbo.api.json` (`WORKFLOW_PIN_I2V_TURBO`)
- `h3.r2v.turbo` → `workflows/h3.r2v.turbo.api.json` (`WORKFLOW_PIN_R2V_TURBO`)

Unknown `model` is an error.

## `h3.t2v`

| RCD / HOST_PAYLOAD | API node | field |
|---|---|---|
| `prompt` | 131 MiniMaxH3ImageToVideo | `prompt` |
| `durationSeconds` | 133 PrimitiveFloat | `value` |
| `fps` | 130 CreateVideo | `fps` |
| fps expression | 132 ComfyMathExpression | `expression` |
| `resolution` | 115 ResolutionSelector | `aspect_ratio` |
| lightning | 139 PrimitiveBoolean | stays `false` |
| output | 92 SaveVideo | history → `/view` |

## `h3.i2v.turbo`

Same timing/prompt nodes as t2v. Lightning `139` is **true**.

| RCD / HOST_PAYLOAD | API node | field |
|---|---|---|
| first frame | upload → 140 LoadImage `image` → 131 `first_frame` | required (`startFrameB64` or `referenceImagesB64[0]`) |

## `h3.r2v.turbo`

| RCD / HOST_PAYLOAD | API node | field |
|---|---|---|
| `prompt` | 138 PrimitiveStringMultiline | `value` |
| `durationSeconds` | 132 PrimitiveFloat | `value` |
| fps expression | 131 ComfyMathExpression | `expression` |
| `fps` | 130 CreateVideo | `fps` |
| `resolution` | 115 | `aspect_ratio` |
| references | upload → 137/139/141 → 136 `ref_images.ref_image_*` | `referenceImagesB64` 1..=3; unused keys removed |
| turbo LoRA | 142 LoraLoaderModelOnly | always on; scheduler steps **8** |

`startFrameB64` without `referenceImagesB64` is an error (not I2V).
