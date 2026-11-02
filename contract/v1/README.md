# 第三方适配器 v1 · 随包作者入口

本目录的 `user-examples/vendor-adapter-v1/` 副本随安装包提供，供用户和编程智能体编写**独立的厂商适配器**。它不是 Creator MCP 客户端，也不是完整开发仓库。安装包中的四个案例均为源码示例，不含可直接导入的 `adapter.wasm`；运行目标厂商仍需自己的服务、凭证和实际调用验证。

## 从哪里开始

1. 先读 [宿主字段契约（中文）](HOST_PAYLOAD.zh-CN.md) 或 [English field contract](HOST_PAYLOAD.en.md)。它们解释宿主的 `payloadJson` 和插件回包；案例不是字段标准。
2. 对照 [WIT 世界](wit/vendor-adapter.wit)、[协议闭集](protocol/vendor-adapter-protocol.json) 和 [JSON Schemas](schemas/manifest.schema.json)。Rust SDK 的用法见 [sdk/rust/README.md](sdk/rust/README.md)。
3. 从 [Rust 起步模板](templates/rust/README.md) 或下方最接近目标厂商的案例开始。把**整个 crate 根目录**复制到可写位置；导入时选含 `Cargo.toml`、`manifest.json`、`wit/`、`vendor-sdk/` 的根目录。
4. 未签名源码包由客户端调用本机 Rust 工具链编译；安装包不附 `rustc`、`cargo-component` 或 `wasm32-wasip2`。预编译并签名的发布包按其自身说明导入。

## 四个源码案例

| 案例 | 用途 |
|------|------|
| [comfyui-lan-minimax-h3](examples/comfyui-lan-minimax-h3/README.md) | 局域网 ComfyUI H3 视频工作流示例。 |
| [comfyui-lan-z-image-turbo](examples/comfyui-lan-z-image-turbo/README.md) | 局域网 ComfyUI Z-Image-Turbo 文生图示例。 |
| [lan-openai-compat-text](examples/lan-openai-compat-text/README.md) | 局域网 OpenAI 兼容文本服务示例。 |
| [video-reference-media-probe](examples/video-reference-media-probe/README.md) | 不连接厂商的参考媒体离线探针。 |

`templates/rust/` 是起步模板，`examples/` 是具体案例。它们不定义宿主字段；字段以 `HOST_PAYLOAD.*.md`、WIT、协议和 schema 为准。案例内地址与密钥可能是占位值，先读对应 README 和配置，再导入自己的副本。

## 升级与边界

- 已有插件升级时，先读 [1.1.0](UPGRADE-1.1.0.md)、[1.2.0](UPGRADE-1.2.0.md)、[1.3.0](UPGRADE-1.3.0.md) 升级说明，并让 WIT、协议、schema 与 Rust SDK 来自同一份随包目录。
- Rust 是此目录提供源码 SDK 的语言。其它语言编译出的合规预编译插件可按宿主支持的导入方式使用；本目录不提供 Python、TypeScript 或 TinyGo SDK。
- 插件的真实厂商请求、鉴权、提交、查询和结果映射由插件自己实现。离线编译或导入成功只证明对应步骤成功，不代表厂商调用或成片已通过。

English: start with [HOST_PAYLOAD.en.md](HOST_PAYLOAD.en.md), [the Rust SDK guide](sdk/rust/README.md), and the README in the example closest to your provider. Copy the complete crate to a writable directory before editing or importing it.

RunningHub 云端案例已迁至独立 SDK v2 工程：[`runninghub-minimax-h3`](https://github.com/IM-DaXiong/bgxiong-ai-story-dev/blob/main/packages/cloud-adapters/runninghub-minimax-h3/README.md)（经典七工作流）与 [`runninghub-h3-dynamic`](https://github.com/IM-DaXiong/bgxiong-ai-story-dev/blob/main/packages/cloud-adapters/runninghub-h3-dynamic/README.md)（动态图音参考）。它们各自构建、检查和打包，源码与安装物均独立提供，不属于本安装包的 V1 SDK 切片。按各工程 README 获取或构建预编译安装包；解压后在「设置 → 第三方适配器 → 安装适配包」选择同时包含 `adapter-install.json` 和 `package/` 的安装根目录，按提示填 API Key。不要选源码目录或单独的 `package/` 子目录；正常安装预编译包无需本机 Rust 工具链。
