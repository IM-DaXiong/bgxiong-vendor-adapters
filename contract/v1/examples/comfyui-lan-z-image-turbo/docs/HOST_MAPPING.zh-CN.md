# HOST_MAPPING · LAN ComfyUI Z-Image-Turbo

字段 SSOT：`packages/vendor-adapter-contract/v1/HOST_PAYLOAD.zh-CN.md`（槽 `image`）。  
字节 SSOT：本 crate `workflows/z.turbo.api.json` + `config.rs` SHA pin。  
禁止猜节点号；换图须对照本表与钉文件。

## 模型

| modelId | 钉文件 | 尺寸 |
|---|---|---|
| `z.turbo` | `workflows/z.turbo.api.json` | 10 档 enum（`payload.size` ↔ RCD `resolution` ↔ `appliedParams.resolution`） |

默认 `1920x1088`（2K 16:9）。画幅是 option 元数据 `aspectRatio`，不另声明 RCD `aspect`。

## Wire → 节点

| Wire | 节点键 | class_type | 字段 | 规则 |
|---|---|---|---|---|
| `prompt` | `57:27` | CLIPTextEncode | `text` | trim 空硬失败 |
| `size` | `57:13` | EmptySD3LatentImage | `width`/`height` | 必须是闭集 WxH；须为 8 的倍数；缺省 1920×1088 |
| （seed） | `57:3` | KSampler | `seed` | 无则由 `client_id` 派生；`steps`/`cfg`/`sampler_name`/`scheduler`/`denoise` 保持钉死 |
| （输出） | `9` | SaveImage | — | query 收 `outputs.images` → `{base}/view?...` |

权重节点（不改）：`57:28` UNET `z_image_turbo_bf16.safetensors`、`57:30` CLIP `qwen_3_4b.safetensors`、`57:29` VAE `ae.safetensors`。

`bindingId` = `57:13.size`。submit 回包 `appliedParams.resolution` = 实际写入的 `WxH`。

## 不宣称

- `negativePrompt`（图为 ConditioningZeroOut；有字段则忽略，不写节点）
- `referenceImagesB64` / i2i（有参考图硬失败；槽未声明图生图）
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
