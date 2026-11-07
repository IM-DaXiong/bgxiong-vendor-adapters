# 本地 ComfyUI 单图扩展全景适配包（1.0.2）

版本以 `manifest.json` 的 1.0.2 为准。下文若仍出现更早的工程描述，不把标题改版写成一次新的实机验收。实跑状态仍以本文「不跑 Comfy 实机 / NOT_RUN」为准。

包 ID：`local.thirdparty.comfy-panorama-image`  
模型：`panorama-image-single`  
操作：`image-to-panorama`  
能力：`image.generate` + `requiredFeatures: ["image-projection-v1"]`  
输出：`equirectangular360`（2048×1024 ERP）

## 范围

- **只改本包**：`packages/workflow-adapters/panorama-image/`。
- **不跑 Comfy 实机**：本机（IM-DaXiong）仅下载模型/冻工作流/写代码；C02/C03/C07–C12 等实跑标 **NOT_RUN**，到另一台机器验收。
- **无旧版兼容层**。

## 工程结构

参考 RunningHub 动态包：独立 Cargo workspace、官方 `bgx-adapter-v2 init` 生成的 `vendor-sdk/` + `template-lock.json`（作者 SDK **2.0.0**）、标准 `Build.ps1`、`installation/adapter-install.template.json`。

> Windows 注意：工作流文件是 [workflow/source.json](workflow/source.json)。不要再放一份只差大小写的溯源文件。溯源摘要在包根 [SOURCE.json](SOURCE.json)。

## 模型（不入 git）

见 `MODEL_ASSETS.json`。本机已落到：

- `E:/bgxiong/data/models/comfyui/diffusion_models/flux-2-klein-base-4b.safetensors`
- `E:/bgxiong/data/models/comfyui/text_encoders/qwen_3_4b.safetensors`
- `E:/bgxiong/data/models/comfyui/vae/flux2-vae.safetensors`
- `E:/bgxiong/data/models/comfyui/loras/flux-2-klein-4B-360-erp-outpaint-lora_V1.safetensors`

自定义节点克隆：`E:/bgxiong/data/comfyui_custom_nodes/ComfyUI-Panorama-Stickers`

## 无头贴图合同

`PanoramaStickers.sticker_state` 固定：

```json
{"kind":"pano_sticker_state","version":1,"pose":{"yaw_deg":0,"pitch_deg":0,"roll_deg":0,"hFOV_deg":90}}
```

SaveImage 终端取 **VAEDecode ERP**，不是 Cutout 透视裁切。

## 构建

SDK 发行基线：CLI/reference-host/retained **2.1.1**（例如 `D:\00Dev\panorama-sdk-2.1.1-candidate`）。

```powershell
# 语法级：原生检查（不启 Comfy）
cargo test --manifest-path Cargo.toml

# 标准包构建（输出目录必须不存在）
.\Build.ps1 -Sdk D:\00Dev\panorama-sdk-2.1.1-candidate -OutputDirectory D:\00Dev\panorama-image-install-out -Online
```

离线二次构建需已有 `.bgx-sdk` 缓存；缺缓存应显式失败，勿静默换 SDK。

## 验收（另一台机器）

1. 按 `MODEL_ASSETS.json` 落模型与节点。  
2. 原生 Comfy 打开 `workflow/source.json`，Export (API) 复核并更新 `SOURCE.json` 摘要（完成 C02）。  
3. 实跑一张场景图（C03/C07）。  
4. 安装 `adapter-install.json` 后走产品 GUI（C08+）。
