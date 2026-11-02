# AGENT.md — 用本仓库开发第三方适配器

你是编码智能体。用户会给你这个仓库，以及某个第三方平台的 API 说明（鉴权、创建任务、查询、下载）和测试用密钥。你的目标是产出可以导入比格熊 / BGXiong 的适配器目录。

本仓库已经带有契约、Rust SDK 和完整示例。按这些材料实现，不要假设还要去另一个仓库找 SDK。

## 成功标准

- [ ] 有合法 `manifest.json`：`schemaVersion` 为 2，并包含 `pluginId`、`displayName`、`version`、`language`、`entry`、`capabilitySlots`、`requestedCapabilities`（对照 `contract/v1/schemas/manifest.schema.json`）
- [ ] Rust guest 导出 WIT `invoke`（`contract/v1/wit/vendor-adapter.wit`，包 `bgxiong:vendor-adapter@1.1.0`）
- [ ] Host 字段按 `contract/v1/HOST_PAYLOAD.zh-CN.md` 映射。不要发明辞典里没有的字段
- [ ] 信封和回包使用本仓库 Rust SDK：`contract/v1/sdk/rust`（示例里是旁边的 `vendor-sdk/`）
- [ ] 异步任务的 `vendorTaskId` 是厂商真实任务号
- [ ] query 的 `failed` / `cancelled` / `expired` 带非空 `terminalFailure`（契约 1.2.0）
- [ ] 密钥只来自用户配置或占位符。仓库里不得出现真实 API Key
- [ ] `python3 scripts/validate-example.py <适配器目录>` 通过
- [ ] 用中文告诉用户：改了哪些文件、要填的密钥名、如何在客户端导入、已知限制

## 必读顺序

1. `docs/SDK接入.md`
2. `contract/v1/HOST_PAYLOAD.zh-CN.md`（英文用 `HOST_PAYLOAD.en.md`）
3. `contract/v1/wit/vendor-adapter.wit`
4. `contract/v1/schemas/manifest.schema.json`
5. `contract/v1/sdk/rust/README.md`
6. `contract/v1/templates/rust/`（新插件从这里复制）
7. 选一个最接近的 `examples/*` 作对照，不要整文件抄成另一家厂商的私货

## 推荐步骤

1. **选标本**
   - 局域网 Comfy 文生图：`examples/comfyui-lan-z-image-turbo`
   - 局域网 Comfy 视频：`examples/comfyui-lan-minimax-h3`
   - OpenAI 兼容文本：`examples/lan-openai-compat-text`
   - 云端异步视频、Bearer：`examples/runninghub-workflow` 或 `examples/runninghub-minimax-h3-r2v`
   - 参考图/视频/音频句柄：`examples/video-reference-media-probe`
2. **建目录**：复制 `contract/v1/templates/rust`，或复制最接近的示例整目录。保留 `wit/` 和 `vendor-sdk/`。
3. **读第三方 API**：列出鉴权、创建、查询、取消、下载，标出同步还是异步。
4. **映射字段**：用 HOST_PAYLOAD 把 prompt、时长、分辨率、参考图对上。未知能力写成明确错误（如 `adapterCapabilityDenied`），不要静默换成别的通道。
5. **实现 guest**：出网只走 `host-http`。不要导入 `wasi:sockets`、`wasi:filesystem`、`wasi:cli`。
6. **写 manifest**：`pluginId` 用 `local.` 前缀的未签名示例风格即可。`entry` 保持 `adapter.wasm`。`requestedCapabilities.endpointPolicy` 与 `auth` 按真实出网方式填写。
7. **脱敏**：示例密钥只用 `REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY` 或 `YOUR_API_KEY`。
8. **校验**：`python3 scripts/validate-example.py <目录>`。本机有 Rust 工具链时再 `cargo test`，以及 `cargo component build --release --target wasm32-wasip2`。
9. **交付导入说明**：设置 → 第三方适配器 → 导入插件包 → 选目录根。然后配置 `baseUrl` 或 Bearer，先探测再试跑。

## 硬约束

- 用户选了哪条通道就跑哪条。禁止静默换厂牌或降级。
- 宿主不读厂商工作流 JSON。可调参数必须在运行时能力文档里声明，并在 `appliedParams` 里回显实际写入厂商请求的值。
- 空 payload 的导入探测（没有 `endpointBaseUrl`）必须成功，并且禁止调用 `host-http`。
- 每个响应只能有 `dataJson` 或 `error` 之一。
- 不要把宿主、UI 或工作站私有运行时源码拷进适配器。
- 不要提交真实 API Key。
- 本仓库许可禁止把仓库内容改造成独立商品出售。交给用户在自己的项目里使用可以。

## 契约版本

以本仓库 `contract/v1` 的 **1.2.0** 为准：WIT `@1.1.0`，manifest `schemaVersion` 2。升级说明在 `contract/v1/UPGRADE-1.1.0.md` 和 `contract/v1/UPGRADE-1.2.0.md`。
