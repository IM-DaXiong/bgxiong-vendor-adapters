# 产品适配器包（packages）

本目录存放可导入比格熊数字导演工作站的**产品级**第三方适配器源码（相对 `examples/` 教学标本更完整）。

当前主线（版本以各包 `manifest.json` 为准）：

- `workflow-adapters/`：`wan22-animate`、`wan-animate2`、`seedvr2-image`、`panorama-image`
- `cloud-adapters/`：DeepSeek / MiniMax M3 / Seedream / RunningHub 系列
- `tts-adapters/`：Breeze（及暂缓独立 SDK 分发的 VoxCPM 材料）
- `comfy-workflow-adapter/`：作者与教学切片
- `third-party/`：独立工程指针（如 H3 Face Refine → `projects/bgxiong-h3-face-refine-adapter`）

构建优先依赖本仓库 `contract/v2/sdk/rust`（见各包 `Cargo.toml`）。部分包另带 `vendor-sdk/` 镜像。不要提交 `target/`、`.bgx-sdk/`、`adapter.wasm`、`__pycache__/`。

本地开发：克隆本开源仓到与主仓 `bgxiong-ai-story` **同级**的 `bgxiong-third-party-adapters`，在包目录构建；构建缓存/安装产物可另放本机产物根（见主仓 GUIDE），Git 真源始终是本仓库。
