# BGXiong Vendor Adapters / 比格熊第三方适配器

开源的第三方算力适配器：**完整案例 + SDK 接入文档**。
下载本仓库后，可对照案例与 SDK 文档，制作可导入[比格熊数字导演工作站](https://www.bgxiong.com)的第三方适配器。

本仓库契约与 SDK 切片同步自产品仓最新 `packages/vendor-adapter-contract`（当前含 **v2 / SDK 2.1.1** 与 v1 兼容树）。

## 怎么用

1. 读 `README.md`（本文件）与 `AGENT.md`
2. 读 `docs/SDK接入.md`（即 SDK-CONTRACT）与 `docs/OPEN-STANDARD.zh-CN.md`
3. 打开 `contract/v2/`（最新）与 `examples/`
4. 按文档实现并打包，在客户端「第三方适配器」导入

## 目录

- `contract/v2` — 最新开放标准、WIT、schemas、SDK、模板
- `contract/v1` — 旧版兼容切片
- `docs/` — SDK 接入与设计文档
- `examples/` — 完整案例（与契约 examples 同步）

仓库地址：https://github.com/IM-DaXiong/bgxiong-vendor-adapters