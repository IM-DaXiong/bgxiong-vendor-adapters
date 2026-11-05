# MiniMax M3 独立文本适配包

使用项目公开SDK v2 2.0.0，SDK源码/合同/CLI/worker不作厂商修改。厂商HTTP协议只在本包。当前首个操作为`text.generate` / `local.bgx.minimax-m3/complete`，模型明确为`MiniMax-M3`。

## 当前范围

普通多轮文本、完整中文原文、缓冲回复、独立思考内容、finish reason及厂商报告的token用量。没有自动换模型、自动重试或异步恢复。工具循环、图片/视频理解、UI逐字流式及M3.1-Flash未在本包开放；不能拿普通文本结果作为这些功能的验证。

参数`maxTokens`映射官方`max_completion_tokens`（1～524288）；`thinking`布尔映射`adaptive`/`disabled`。M3温度实际固定1，因此只接受1，其他值明确报错。不接受未消费的topP/JSON模式等字段，不静默剥离。

结果outputs为一项answer文本；data的assistantMessageJson保留完整assistant消息JSON，另有模型、响应ID、finishReason和reasoningContent；usage为reported measurements，缺失就null，不估算。`length`或`content_filter`保留原状态，不假称完整结束。原始厂商消息作为不透明JSON文本保留，宿主不依赖其厂商字段，不扩大SDK结构合同。

官方核对日期2026-10-25：[OpenAI兼容](https://platform.minimax.cn/docs/api-reference/text-openai-api)、[对话补全合同](https://platform.minimax.cn/docs/api-reference/text-chat-openai)。默认服务origin可用`https://api.minimax.cn`，精确请求路径为`/v1/chat/completions`；国际区应按该账户官方地址显式配置，不自动轮换地区。

## 构建和接入

本工程仅依赖公开SDK Rust包及锁定registry依赖。使用固定CLI依次`build --sdk <SDK> --project <本工程>`、`check --sdk <runtime-sdk> --worker <worker> --project <本工程>`、`pack --sdk <SDK> --project <本工程> --out <不存在的输出目录>`，再对打包目录执行同样的check。

从capabilities读取真实schema摘要和操作身份。host-config显式配置`endpointBaseUrl`（origin，不带/v1）、`credentialRef`；network.origins仅授权实际origin，生产不需开启私网。凭证由host.credentials受控提供，guest只获得引用，SDK负责Bearer注入；不得在本包放API Key。构建/包检查只证明包合法，不证明厂商连接或计费请求成功。

提交input示例：

```json
{"messages":[{"role":"system","content":"简洁回答。"},{"role":"user","content":"你好"}],"parameters":{"thinking":false,"maxTokens":32}}
```

外层使用SDK公开submit信封，modelId为`MiniMax-M3`，capabilityKey为`text.generate`，operationId为`local.bgx.minimax-m3/complete`，operationVersion为1，inputSchemaDigest取当前capabilities原值，media/uploads均空数组。每次调用使用独立requestId。

2026-10-25，1.0.2已通过固定SDK的真实厂商调用；0.6.1101开发版还完成软件标准安装、加入普通文本模型、选择此包及普通聊天真实回复。软件使用隔离验收profile和显式短期凭据，测试后已删除密钥文件并恢复临时DNS调整；这不代表生产账号自动绑定或新安装器已验。实际构建、HTTP夹具、厂商实跑和软件验收回执见[主计划](../../../docs/dev/plans/20261025-PLAN-SDKv2云端文本模型独立适配与MiniMaxM3接入.md)。
