# 比格熊第三方适配器 / BGXiong Third-Party Adapters

克隆本仓库之后，按完整示例和 SDK 接入文档，就可以开发能导入 **比格熊数字导演工作站（BGXiong）** 的第三方适配器。

官网：<https://www.bgxiong.com>

## 这是什么

本仓库是第三方适配器的公开开发包：

- 完整示例：[`examples/`](examples/)
- SDK 接入文档：[`docs/SDK接入.md`](docs/SDK接入.md)
- 公开契约：WIT、JSON Schema、Host 字段辞典、Rust SDK、Rust 模板，都在 [`contract/v1/`](contract/v1/)

适配器是工作站加载的 guest 插件。工作站通过 WIT 的 `invoke` 把一次调用交给插件；插件用 `host-http`、`host-media`、`host-credential` 访问厂商。密钥放在使用者本机的适配器配置里，不要写进仓库。

## 版本

| 项 | 值 |
|---|---|
| 契约发布 | **1.2.0**（[`contract/v1/protocol/vendor-adapter-protocol.json`](contract/v1/protocol/vendor-adapter-protocol.json) 的 `contractRelease`） |
| 目录 | `contract/v1`（协议主版本 1，`protocolHostMajor = 1`） |
| WIT | `bgxiong:vendor-adapter@1.1.0`。1.2.0 收紧的是 query JSON 里的 `terminalFailure`，不是新的 WIT ABI |
| Manifest | `schemaVersion: 2`（`VendorAdapterManifestV2`） |
| Rust SDK | [`contract/v1/sdk/rust`](contract/v1/sdk/rust)，crate `bgx-vendor-adapter-sdk` 1.2.0 |

按本仓库里的这些文件开发、校验、导入。

## 怎么用

1. 克隆本仓库。
2. 先读 [`AGENT.md`](AGENT.md)。交给编码智能体时，把本仓库和这份手册一起给它，并附上目标平台的 API 说明。
3. 从下表选一个最接近的示例，阅读整个目录（含 `wit/`、`vendor-sdk/`、`manifest.json`），不要只抄 `src/`。
4. 按 [`docs/SDK接入.md`](docs/SDK接入.md) 对照字段辞典 [`contract/v1/HOST_PAYLOAD.zh-CN.md`](contract/v1/HOST_PAYLOAD.zh-CN.md)。新插件从 [`contract/v1/templates/rust`](contract/v1/templates/rust) 复制，或在示例上改。
5. 校验：`python3 scripts/validate-example.py <适配器目录>`
6. 在比格熊里导入该目录：**设置 → 第三方适配器 → 导入插件包**。选中的根目录必须同时有 `manifest.json`、`Cargo.toml`、`wit/`、`vendor-sdk/`、`src/`。
7. 在客户端填写厂商地址和密钥，先探测，再试跑。

导入源码包的电脑需要 `rustc`、`cargo`、`cargo-component` 和 target `wasm32-wasip2`。客户端会执行：

```bash
cargo component build --release --target wasm32-wasip2
```

这些工具不在桌面安装包里，需要本机安装。

## 示例

| 目录 | 做什么 | 密钥 |
|---|---|---|
| [`examples/comfyui-lan-z-image-turbo`](examples/comfyui-lan-z-image-turbo) | 局域网 ComfyUI 文生图（Z-Image Turbo） | 无。地址可配，标本默认 `http://192.168.18.8:8188` |
| [`examples/comfyui-lan-minimax-h3`](examples/comfyui-lan-minimax-h3) | 局域网 ComfyUI 视频（MiniMax H3 文生视频 / 图生视频 / 多参考） | 无。地址可配 |
| [`examples/lan-openai-compat-text`](examples/lan-openai-compat-text) | 局域网 OpenAI 兼容文本 | 可选 Bearer。没有 `endpointBaseUrl` 时不会用标本 IP |
| [`examples/runninghub-workflow`](examples/runninghub-workflow) | RunningHub 云端视频工作流 | 占位符 `REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY`，换成自己的 Key |
| [`examples/runninghub-minimax-h3-r2v`](examples/runninghub-minimax-h3-r2v) | RunningHub 上的 MiniMax H3 多参考视频（1–4 槽） | 同上，占位符，须换成自己的 Key |
| [`examples/video-reference-media-probe`](examples/video-reference-media-probe) | 离线参考媒体探活（特性 `video-reference-media-v1`） | 无 |

示例只供对照。字段标准是 Host 字段辞典 + WIT + schemas + Rust SDK。

`contract/v1/examples/` 是同一套标本的副本，让契约目录可以单独阅读。平时从 `examples/` 进入。

## 目录

```text
README.md                 本文件
AGENT.md                  编码智能体手册
docs/SDK接入.md           SDK 接入文档（中文）
docs/SDK-INTEGRATION.en.md
contract/v1/              契约 1.2.0：字段辞典、WIT、schemas、Rust SDK、模板、fixtures
examples/                 可导入的完整标本
scripts/validate-example.py
LICENSE                   自定义许可（不是 MIT）
NOTICES.md
```

## English

Clone this repository and follow the examples plus the SDK integration doc to build a third-party adapter that imports into the BGXiong workstation.

1. Read [`AGENT.md`](AGENT.md).
2. Pick an example under [`examples/`](examples/).
3. Follow [`docs/SDK-INTEGRATION.en.md`](docs/SDK-INTEGRATION.en.md) and the field dictionary [`contract/v1/HOST_PAYLOAD.en.md`](contract/v1/HOST_PAYLOAD.en.md).
4. Run `python3 scripts/validate-example.py <adapter-dir>`.
5. In BGXiong: Settings → Third-party adapters → Import package. Select the adapter root (`manifest.json`, `Cargo.toml`, `wit/`, `vendor-sdk/`, `src/`).

Published contract in this tree: **release 1.2.0**, WIT `bgxiong:vendor-adapter@1.1.0`, manifest `schemaVersion` 2. The Rust SDK is [`contract/v1/sdk/rust`](contract/v1/sdk/rust).

## 许可

自定义许可，不是 MIT。全文见 [LICENSE](LICENSE)。
