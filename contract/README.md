# 公开契约 / Public contract

比格熊第三方适配器的公开契约切片，契约发布 **1.2.0**。

目录名 `v1` 是协议主版本（`protocolHostMajor = 1`，WIT `bgxiong:vendor-adapter@1.1.0`）。Manifest 使用 **schemaVersion 2**（`VendorAdapterManifestV2`）。1.2.0 收紧的是 query 回包里的 `terminalFailure`，不是新的 WIT ABI。见 [v1/UPGRADE-1.2.0.md](v1/UPGRADE-1.2.0.md)。

开发时以本仓库这些文件为准。接入步骤见仓库根目录 [docs/SDK接入.md](../docs/SDK接入.md)。

## 从这里读

1. 字段辞典
   - 中文：[v1/HOST_PAYLOAD.zh-CN.md](v1/HOST_PAYLOAD.zh-CN.md)
   - English：[v1/HOST_PAYLOAD.en.md](v1/HOST_PAYLOAD.en.md)
   - 索引：[v1/HOST_PAYLOAD.md](v1/HOST_PAYLOAD.md)
2. WIT：[v1/wit/vendor-adapter.wit](v1/wit/vendor-adapter.wit)
3. Rust SDK：[v1/sdk/rust](v1/sdk/rust)
4. 模板：[v1/templates/rust](v1/templates/rust)
5. Schemas：[v1/schemas](v1/schemas)
6. 闭集：[v1/protocol/vendor-adapter-protocol.json](v1/protocol/vendor-adapter-protocol.json)

`v1/examples/` 与仓库根 `examples/` 是同一套标本，不是字段标准。
