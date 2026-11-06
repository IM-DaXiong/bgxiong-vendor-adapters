# AGENT.md — 用本仓库开发第三方算力适配器

你是编码智能体。用户会给你：(1) 本仓库；(2) 某个第三方算力平台的 API 文档与凭证。你的目标是产出**可导入比格熊 / BGXiong 的第三方适配器包**。

## 成功标准

- [ ] 存在合法 `manifest.json`（通过 `contract/v1/schemas/manifest.schema.json` 字段约束）
- [ ] 实现 WIT `contract/v1/wit/vendor-adapter.wit` 要求的 guest 导出（以模板为准）
- [ ] Host 字段映射遵循 `HOST_PAYLOAD.zh-CN.md` / `.en.md`（勿发明未文档化字段）
- [ ] 密钥只来自用户配置/环境变量占位符，**禁止**把真实密钥写进仓库
- [ ] README 说明鉴权、创建任务、轮询、取结果的路径
- [ ] `python scripts/validate-example.py <pack-dir>` 通过

## 必读顺序

1. [README.md](contract/README.md)
2. [HOST_PAYLOAD.zh-CN.md](contract/v1/HOST_PAYLOAD.zh-CN.md)（英文用 [HOST_PAYLOAD.en.md](contract/v1/HOST_PAYLOAD.en.md)）
3. [vendor-adapter.wit](contract/v1/wit/vendor-adapter.wit)
4. [manifest.schema.json](contract/v1/schemas/manifest.schema.json) + 同目录其它 schemas
5. [README.md](contract/v1/sdk/rust/README.md)（Python / TypeScript / TinyGo 见各自 README，状态是暂不支持）
6. [rust](contract/v1/templates/rust/)（从这里复制开工）
7. 选一个最接近的 [examples](examples/) 作对照（不要整文件抄业务私货）

SDK V2 在 [README.md](contract/v2/README.md)，目前是开发者预览：[`release-lock.json`](contract/v2/release-lock.json) 的 `sdkReady` 为 false。不要把预览文档当成已稳定发布的唯一接口。产品级适配器清单见 [README.zh-CN.md](packages/README.zh-CN.md)。

## 推荐实现步骤

1. **建包目录**：从 `templates/rust` 复制为 `adapters/<vendor-slug>/`（或用户指定路径）。
2. **读第三方 API**：列出鉴权、创建、查询、取消、下载；标出同步/异步。
3. **映射到 Host 字段**：用 HOST_PAYLOAD 词典把 prompt、时长、分辨率、参考图、回调等对上；未知能力写清「不支持」而非静默吞掉。
4. **实现 guest**：在 `src/` / `vendor-sdk` 内完成 HTTP 调用；错误返回可读信息。
5. **写 manifest**：`id`、能力槽、运行时 caps、信任相关字段按 schema。
6. **脱敏**：示例里只用 `YOUR_API_KEY` 等占位符。
7. **校验**：跑 `scripts/validate-example.py`；有 Rust 工具链则按模板 README 构建 `wasm32-wasip2`。
8. **交付**：告诉用户如何在 BGXiong 客户端导入，以及需要配置的密钥名。

## 硬约束

- 禁止静默换通道 / 降级到未声明的后端。
- 禁止把产品 Host 源码拷进适配器。
- 禁止提交真实 API Key。
- 本仓库许可禁止「改造后当独立商品售卖」；交付给用户自用/商用集成可以。

## 输出给用户时

用中文说明：改了哪些文件、如何配置密钥、如何导入客户端、已知限制与联调清单。
