# DEVELOPING — 适配器开发指南

## 契约入口

| 文档 | 用途 |
|------|------|
| `contract/v1/HOST_PAYLOAD.zh-CN.md` | Host 下发/回收字段词典 |
| `contract/v1/wit/vendor-adapter.wit` | Guest WIT |
| `contract/v1/schemas/*.schema.json` | manifest / caps / profile 等 |
| `contract/v1/sdk/rust` | Rust SDK |
| `contract/v1/templates/rust` | 最小可编译模板 |

## 建议流程

模板复制 → 映射第三方 API → 实现 → 填 manifest → validate → 构建 wasm → 客户端导入联调。

细节与字段语义以 HOST_PAYLOAD 为准；`examples/` 仅标本。
