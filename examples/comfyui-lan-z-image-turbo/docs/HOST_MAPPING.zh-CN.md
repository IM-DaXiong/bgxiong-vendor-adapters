# HOST_MAPPING · LAN ComfyUI Z-Image-Turbo

字段 SSOT：`packages/vendor-adapter-contract/v1/HOST_PAYLOAD.zh-CN.md`（槽 `image`）。  
字节 SSOT：本 crate `workflows/*.api.json` + `config.rs` SHA pin。  
禁止猜节点号；换图须对照本表与钉文件。

## 模型

| modelId | 钉文件 | 原生 latent |
|---|---|---|
| `z.turbo` | `workflows/z.turbo.api.json` | 1920×1088 |
| `z.turbo.1080` | `workflows/z.turbo.1080.api.json` | 1920×1088（与 turbo 同分辨率仍分档） |
| `z.turbo.4k` | `workflows/z.turbo.4k.api.json` | 3840×2160 |

## Wire → 节点（三模型键名相同）

| Wire | 节点键 | class_type | 字段 | 规则 |
|---|---|---|---|---|
| `prompt` | `57:27` | CLIPTextEncode | `text` | trim 空硬失败 |
| `size` | `57:13` | EmptySD3LatentImage | `width`/`height` | 缺省用原生；若传入须等于原生 WxH，否则硬失败 |
| （seed） | `57:3` | KSampler | `seed` | 无则由 `client_id` 派生；`steps`/`cfg`/`sampler_name`/`scheduler`/`denoise` 保持钉死 |
| （输出） | `9` | SaveImage | — | query 收 `outputs.images` → `{base}/view?...` |

权重节点（不改）：`57:28` UNET `z_image_turbo_bf16.safetensors`、`57:30` CLIP `qwen_3_4b.safetensors`、`57:29` VAE `ae.safetensors`。

## 不宣称

- `negativePrompt`（图为 ConditioningZeroOut；有字段则忽略，不写节点）
- `referenceImagesB64` / i2i（有参考图硬失败）
- 视频 / duration / fps

## HTTP

| 步骤 | 路径 |
|---|---|
| probe（仅有 `endpointBaseUrl`） | `GET /system_stats` |
| submit | `POST /prompt` body `{ prompt, client_id }` |
| query | `GET /history/{prompt_id}` |
| 结果 | `/view?filename=…&type=…` |

空 probe（导入 L3）→ `{ok:true,live:false}`，禁止 host-http。  
默认 `COMFY_BASE_URL=http://192.168.18.8:8188` 仅用于 submit/query 回落。
