# LAN ComfyUI · Z-Image-Turbo（文生图第三方适配器）

独立 guest 插件（`pluginId=local.example.comfyui-lan-z-image`）。**不是**内置 `image_comfyui`，**不是** MiniMax H3 视频插件。

执行体：用户局域网 Comfy HTTP。

| 项 | 值 |
|---|---|
| 默认地址 | `http://192.168.18.8:8188`（可随包；凭证 `baseUrl` 可覆盖） |
| 模型 | `z.turbo` / `z.turbo.1080` / `z.turbo.4k` |
| 工作台 | 文生图选择器 `adapter:local.example.comfyui-lan-z-image:{modelId}` |
| HTTP | `GET /system_stats` · `POST /prompt` · `GET /history/{id}` · `/view` |
| 无 | `/upload/image`、图生图、负面提示词可调 |

## 导入

1. 设置 → 第三方适配器 → 导入插件包 → 选**本目录根**（含 `Cargo.toml` + `manifest.json` + `wit/` + `vendor-sdk/` + `src/` + `workflows/`）
2. 本机需 T0：`rustc` / `cargo` / `cargo-component` / `wasm32-wasip2`
3. 导入成功后重启客户端；凭证可填或保持默认 `http://192.168.18.8:8188`
4. 打开**文生图**工作台（image / t2i workbench），选上述三模型之一

作者改图前先读 `src/config.rs` 顶部 `AI_AUTHOR_GUIDE` 与 `docs/HOST_MAPPING.zh-CN.md`。

L2 hello 绿 ≠ 导入 L3 绿 ≠ 可生成。远端须已装 Z-Image 权重（`z_image_turbo_bf16` / `qwen_3_4b` / `ae`）。CI 禁止写「已接通」。

## 节点映射

见 [`docs/HOST_MAPPING.zh-CN.md`](docs/HOST_MAPPING.zh-CN.md)。

## 单测

```text
cargo test --manifest-path packages/vendor-adapter-contract/v1/examples/comfyui-lan-z-image-turbo/Cargo.toml
```
