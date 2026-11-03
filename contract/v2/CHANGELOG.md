# 变更记录

## 2.0.2 工具／参考宿主 S0 候选 — 2026-10-26

- 发布 speech-v1 三操作 schema、固定摘要与三层数据类型；缺 UI 只给诊断，不阻断合法调用，媒体边界继续由 schema 校验。
- 公共 HTTP 出站前持久化；CLI 内部日志与可选 `--host-channel-v1` 确认。当前只协商 before-send。
- `init --template speech` 沿用 build/check/pack；修正作者模板把工具补丁号误当合同版本的问题。模板不包含模型推理。
- 核心协议/WIT 仍为 2.0.0，语音 profile 为 v1；候选由提交、公共 schema 摘要、模板镜像锁和实际工具摘要固定。全 SDK 发行治理与应用接入状态不因此变为通过。详见 [Speech v1](profiles/speech-v1/README.md)。

## 2.0.0 开发候选准备稿 — 2026-10-19 — Unreleased

本条记录标准 2.0.0 的公开开发实现，不是已发布版本、可分发授权或最终 SDK_READY 声明。完整候选导出、三类服务/样例矩阵、隔离作者、复现和最终审查仍须对应真实证据。此段保留2026-10-19独立候选的历史范围；2026-10-24预览安装版已支持工作流关联的v2包导入，当前范围以 [预览版说明](DEVELOPER-PREVIEW.zh-CN.md) 为准。

### 合同与作者接口

- 六种传输操作仍为 capabilities/probe/upload/submit/query/cancel，复用 Invocation/Response。业务 operationId/version 与传输 operation 分离；generic 能力和新增 namespaced 操作/参数通过公开 schema 声明，不增加厂商或模型特判。
- 动态能力、模型、多操作、configurationSchema、严格 input/output/resumeSchema、UI 控件、媒体/输出槽、真实 usage、appliedParams 与生命周期使用同一套公开定义。字段参考[SDK 合同](SDK-CONTRACT.zh-CN.md)、[分派表](operation-contracts.json)和[schemas](schemas/)。
- schema/类型/WIT 同源生成；schemaJson 的 digest 取原始解码字符串 UTF-8 字节，拒绝未知约束、越权/缺失引用、递归和摘要漂移。Rust no_std 作者接口支持 native 测试及 wasm32-wasip2；原生完整验证器独立复用成熟 schema 引擎。
- 身份、namespace、签名、required feature 与权限前置验证；签名使用 inventory 实际摘要和明确可信发布者，不接受 trusted 布尔值。当前 LicenseRef-Proprietary 标识不因接口公开而变更。

### 独立执行实现

- 独立 workspace、WASM component worker、严格进程帧、资源预算/超时/取消/回收，以及统一 ReferenceHost 六操作内核。公开 WIT 提供十八个受管端口，范围见[宿主指南](HOST-INTEGRATION.zh-CN.md)；不读取主程序配置或数据库。
- 媒体不可变身份、租约、分块读写/上传下载、真实格式/扩展名、显式 ffprobe、实际 VFR 时间戳；checkpoint 保存可重建内容身份而非长期 handle。
- URL/DNS/私网与凭据授权经原 broker；不使用环境 proxy、不自动跟随重定向、不自动重试。凭据来自显式开发 secretFile，guest 不得到原 key。
- 可选 stream-events-v1 同时包含真实 HTTP pull stream 和有序事件；消费 ACK/背压与唯一最终 Response，EOF 不算成功。真实 usage 快照不重复累加，缺报告保持 null。
- submit 前持久锚点，accepted 在对外交付前原子保存；重启 query 固定原包/配置/schema/task，不重新 submit。不确定受理、local_only 与 remote_requested/remote_confirmed 分别表达，不自造任务或取消事实。

### 作者工作链

- init/build/check/pack 使用明确 SDK/项目/worker 路径。init 白名单与模板镜像封存；作者 Cargo.lock 可显式维护，编译仍锁定公共依赖。作者私有 Cargo cache、无隐式配置、受控 OS 工具链发现及源码路径 remap；离线前须显式联网缓存。
- check 实际执行 capabilities/package_validation 且关闭网络/凭据。pack 只输出运行清单/入口/签名；拒绝覆盖目标、篡改 inventory 和无效信任。签名明确指定私钥及密码文件或未加密模式，不猜测模式、不重建已签包。
- 六操作 CLI 使用完整公开 Invocation 和封闭的显式 provisioning 配置，持久回执/事件/诊断，另进程 query/cancel 原任务。精确命令及独立 echo 示例见[作者指南](AUTHORING.zh-CN.md)。
- Rust/WASM record/replay 会实际执行 guest 与精确端口 tape，离线重放没有真实网络 broker；记录明确为 MOCK，秘密/签名交互不能打码后伪造为有效 tape。conform 保留 EVIDENCE_DECLARATION 与 ACTUALLY_EXECUTED 的区别，缺 required case 不通过。
- 配置型 profile 为原生安全解释器，通过同一 ReferenceHost/session/validator。JSON pointer/type witness、静态映射和运行时远端 JSON 检查、显式凭据 marker 以[profile semantics](protocol/profile-semantics.json)为准；运行类型为 profile，不是 WASM。streaming、binary upload、租约媒体输出、复杂签名/条件工作流及 profile tape 暂不支持，明确拒绝而不回退。
- 参考表单源码供独立本地导入/呈现/导出和回执查看；不自动请求模型、授予媒体租约或把浏览器展示当原生验证。三类独立 loopback 服务及可执行样例仍须后续按同一 runner/hash 实跑并标 MOCK，不能把已有设计 JSON 或编译代替执行。

### 破坏性变化与接入边界

v2 是主版本合同，不能用 v1 crate/worker 接受新增字段，不能以 alias、缺字段默认、第二参数袋或运行失败后退回 v1 维持两条路径。同一生产切换单元的 GUI/F1/MCP/批处理入口须另行完整迁移；未接入模块继续其既有完整实现，不会因公开 SDK 编译而自动改变。

本记录没有关闭既有生产能力、安装新版生产 worker、执行用户数据迁移或宣告 MODULE_READY。SDK_READY 也不能替代模块自己的 G07。

### 尚未完成的发行事项

- 候选 export_sdk.py、verify_release.py、check_author_evidence.py 的发行工作链尚未完成验收；本条不将它们列为当前 bgx-adapter-v2 CLI 的可用子命令。
- 公开 release-manifest/release-lock、依赖源码与许可证完整清单、阻断主仓访问后的候选构建/执行、隔离作者新增操作/参数、两次实际候选构建及最终自审仍需独立材料。
- G00–G06 与 C01–C36 以真实分阶段验收结果为准。局部自测/子矩阵不是完整门通过；C35/C36 后续证据缺失时仍 required/NOT_RUN，不能伪报全绿。
- 不声称任何未实测真实厂商 LIVE 可用。形式可解析、离线缓存可用、参考宿主或浏览器可打开，都不等于最终独立开箱验收完成。

正式发行时应以真实版本、候选摘要和完成证据更新本节；本文不创建版本 tag、不推送或发布任何远端产物。
