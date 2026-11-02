# AGENT.md — 用本仓库开发第三方算力适配器

目标：产出可导入比格熊 / BGXiong 的第三方适配器包。

## 必读顺序

1. `docs/OPEN-STANDARD.zh-CN.md`
2. `docs/SDK接入.md`
3. `contract/v2/wit/vendor-adapter.wit` 与 `contract/v2/schemas/`
4. `contract/v2/templates/` 与 `examples/`
5. `docs/AUTHORING.zh-CN.md`

以 **contract/v2** 为准；v1 仅作兼容参考。禁止提交真实 API Key。禁止把产品 Host 源码拷进适配器。