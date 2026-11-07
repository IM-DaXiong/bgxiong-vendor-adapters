# DEVELOPING — 适配器开发指南

## 契约入口

| 文档 | 用途 |
|------|------|
| [HOST_PAYLOAD.zh-CN.md](../contract/v1/HOST_PAYLOAD.zh-CN.md) | Host 下发/回收字段词典 |
| [vendor-adapter.wit](../contract/v1/wit/vendor-adapter.wit) | Guest WIT |
| [schemas](../contract/v1/schemas) | manifest / caps / profile 等 |
| [README.md](../contract/v1/sdk/rust/README.md) | Rust SDK（当前唯一 supported 的语言 SDK） |
| [rust](../contract/v1/templates/rust) | 最小可编译模板 |
| [README.md](../contract/v1/sdk/python/README.md) | Python：暂不支持，没有可运行库 |
| [README.md](../contract/v1/sdk/typescript/README.md) | TypeScript：暂不支持，没有可运行库 |
| [README.md](../contract/v1/sdk/tinygo/README.md) | TinyGo：暂不支持，没有绑定 |

上表路径都是本仓库里的可点击入口。Rust 是目前唯一带库代码的 V1 语言 SDK。

## SDK V2（开发者预览）

从 [README.md](../contract/v2/README.md) 进入。预览边界见 [`DEVELOPER-PREVIEW.zh-CN.md`](../contract/v2/DEVELOPER-PREVIEW.zh-CN.md)，接口见 [`SDK-CONTRACT.zh-CN.md`](../contract/v2/SDK-CONTRACT.zh-CN.md)，作者步骤见 [`AUTHORING.zh-CN.md`](../contract/v2/AUTHORING.zh-CN.md)。[`release-lock.json`](../contract/v2/release-lock.json) 的 `sdkReady` 为 `false`。预览样例不能写成已封存发行。

## 建议流程

模板复制 → 映射第三方 API → 实现 → 填 manifest → validate → 构建 wasm → 客户端导入联调。

细节与字段语义以 [HOST_PAYLOAD](../contract/v1/HOST_PAYLOAD.zh-CN.md) 为准；[examples](../examples/) 仅标本。

仓内 Markdown 相对链接用 `python scripts/check-doc-links.py` 检查。说明见 [DOC-LINKS.md](DOC-LINKS.md)。
