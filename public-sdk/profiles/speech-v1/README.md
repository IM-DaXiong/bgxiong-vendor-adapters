# Speech v1：S0 冻结候选

本目录是三层语音合同的机器表示。语义依据项目 `docs/dev/specs/20261026-SPEC-TTS三层对外合同与SDK接入要求.md`。公开协议仍为 V2 / WIT 2.0.0；本次 CLI、参考宿主为 2.0.2。它是 S0 开发候选，不能代表主程序已经支持 TTS 包或完整 SDK 发行验收已完成。

- 主程序—语音模块：`host.schema.json`，包含领域输入、输出及 `Reply`；这里只发布数据形状，没有 IPC/Creator 实现。
- 语音模块—adapter：`synthesize.*`、`batch.*`、`voices.*`、`resume`、`descriptor`、`configuration`。
- 包—受管 runtime：同级 `../local-runtime-v1/`，只发布 bundle、启动和控制数据形状，没有安装或启动实现。放在 SDK 内使公开作者源码包含完整生成依赖。
- 宿主工具通道：`../../schemas/host-channel-v1.schema.json`。S0 只协商 `before-send`；媒体桥消息有 schema，执行留到 S4。

最小输入为 `{"schemaVersion":1,"text":"你好。"}`；mode 缺省 plain，execution.delivery 缺省 complete。schema 的 default 是合同注释，验证器不会替调用方修改输入。实际语音绑定、未应用参数回执和模型转换由后续阶段按合同实现，不在 UI 再造规则。

成功合成必须有 SDK 原有的真实 `role=audio` 输出；`completed.data=null` 合法。附件是独立可选数据，外层接受 `attachments: Json[]`，每项按 synthesize.output 的 `$defs` 独立解析；坏附件只产生提示，不拒收有效音频。空 UI、未知 descriptor、没有流/批量/克隆/目录/性能数据都不是普通合成的前置条件。权限、真实媒体引用、schema 类型和公共操作摘要仍需校验。

三个 std 操作的固定 input/output/resume 摘要在 `../../protocol/vendor-adapter-protocol.json`。同步包也引用同一 resume schema，但不能因此声称可恢复。只有包实现的操作才应出现在 RCD：作者模板只声明 synthesize，fixture 的三操作仅用于合同校验。

生成命令：

```powershell
python tools/generate_contracts.py
python tools/generate_wit.py
python tools/sync_templates.py --write
```

现有生成器产生 Rust/TypeScript 类型、fixture 与 speech 模板的公共 schema 字节；`--check` 检查生成漂移。std 原始字节与已冻结摘要不符会停止生成，不自动改 registry 来掩盖合同变化。模板镜像继续使用既有 template-lock。

CLI：`init --template speech` 后使用原有 build/check/pack。第一次仓外 build 需要准备锁定依赖；之后可用 `--offline`。模板未连接模型时 submit 明确报错，不能用于产品声音验收。

出站审计：独立 CLI 在受管 workspace 的`cli/<requestId的SHA-256>/` 子目录写 `before-send.jsonl`，flush/sync 成功后才出站。指定 `--host-channel-v1` 时先握手接受 before-send，再对每条记录等同 exchangeId 的 `audit_ack.result.persisted=true`；拒绝、失联或超时不发起该 HTTP。最终仍返回原 SDK Response，完整媒体沿现有 retained 导出路径接管。应用 logs/ai_tasks 双写尚未接入。

审计默认保留 JSON/UTF-8 正文的全部文字、multipart 的原顺序；二进制和媒体记录实际摘要/长度，不写凭据头，broker 已知秘密脱敏。`HttpRequest.audit` 自定义投影尚不接受：它会改变当前 typed WIT，按用户要求留到可选增强，基础审计不依赖它。
