# 产品适配器包（packages）

本目录存放可导入比格熊数字导演工作站的**产品级**第三方适配器源码（相对 `examples/` 教学标本更完整）。

当前包含：

- `workflow-adapters/wan22-animate`：Wan 2.2 Animate 人物替换（ComfyUI）+ 跟踪开箱能力（`video-target-keyframes-v1`），版本见包内 `manifest.json`。

构建依赖本仓库 `contract/v2/sdk/rust`（见各包 `Cargo.toml` 相对路径）。不要提交 `target/`、`.bgx-sdk/`、`adapter.wasm`、`__pycache__/`。

本地开发：克隆本开源仓到与主仓 `bgxiong-ai-story` **同级**的 `bgxiong-vendor-adapters`，在包目录构建；构建缓存/安装产物可另放本机产物根（见主仓 GUIDE），Git 真源始终是本仓库。
