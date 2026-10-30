# 独立宿主接入指南

合同2.0.0。本页说明独立参考宿主的装配方法；当前安装版已支持的ComfyUI接入见 [预览版说明](DEVELOPER-PREVIEW.zh-CN.md)。独立SDK完整治理的NOT_ADOPTED初始化状态不否定这些产品入口，也不替代其验收。本页不授权修改生产配置、数据库或安装目录；SDK_READY与模块MODULE_READY/G07分别验收。

先读[开放标准](OPEN-STANDARD.zh-CN.md)、[输入/结果与作者 API 合同](SDK-CONTRACT.zh-CN.md)和[作者命令指南](AUTHORING.zh-CN.md)。协议字段以[schemas](schemas/)、[operation-contracts.json](operation-contracts.json)、[host ports schema](schemas/host-ports.schema.json)及[WIT](wit/vendor-adapter.wit)为唯一来源。当前没有另一本独立 SDK-API 文件；可调用 API 由 [Rust SDK](sdk/rust/src/lib.rs)、[Host](sdk/rust/src/host.rs)、[validator 导出](crates/validator/src/lib.rs)、[ReferenceHost](crates/reference-host/src/dispatch.rs)和[transport](crates/transport/src/lib.rs)提供。

## 1 装配同一条执行链

参考宿主已实现包校验、目录、输入/结果验证、媒体、网络、凭据、事件、checkpoint、恢复与取消。优先直接使用这些公共模块；不要在 CLI、表单、GUI、MCP 各写一套参数校验或状态机。

1. 用 config::LoadedPackage::load 显式读取运行包，并使用显式 TrustContext 验证 inventory/签名/版本/功能/身份。先测真实入口字节，不能把一个 trusted 布尔值当验签。
2. 用 capabilities::Binding::new 建立包身份、配置、绑定 ID/revision 的绑定。configuration 只有非秘密数据；configuration_ref 是稳定的宿主配置引用。
3. 构造 ReferenceConfig：独立 DevelopmentWorkspace、公开 SDK 根、该根内的 worker、传输预算、网络授权、CredentialProvision、媒体 TTL 和可选 ProbeTool。没有自动扫描配置或默认凭据来源。
4. 用 ReferenceHost::new 装配，capabilities 真实调用后读取 catalog()。submit/upload 持显式 CapabilityRef；query/cancel 持最初 submit 的 checkpoint 身份，复用冻结声明，不重新选择其他能力。
5. invoke/invoke_observed 消费同一个 Invocation、Control 和可选 EventSink；成功后仍须 wait_idle() 确认受管资源完成清理。丢弃 future 也经过共享清理认领，不能把 drop 当远端取消。

ReferenceHost 按 manifest.language 在 Rust/WASM 和 profile 两种作者运行方式间选择一次。profile 仍经同一个 prepare/finish、session、事件完成、校验器与清理；不调用另一套业务调度。profile 回执 worker_pid=None，运行类型为 profile。profile 暂不支持观察器 tape 录制/重放，明确拒绝；不能用 WASM 的证据格式覆盖这个缺口。

WASM 作者只实现 Adapter::invoke(Invocation, &mut dyn Host) -> Response，并通过 export_adapter! 导出；所有作用经 Host::call。原生异步装配通过 transport::ManagedHost::call 和 PortFuture 连接已实现 broker。完整 JSON Schema 引擎在原生 validator 中，不强制塞进 no_std guest。生成的 WIT 枚举与 JSON 枚举须经 SDK 的生成 ABI 转换，不能自行猜 v0/v1 的业务意义。

## 2 十八个受管端口

这些名称来自 host-ports.schema/WIT；参数及返回值使用生成类型。实现方必须在产生作用前检查权限、scope、预算与 Control。

- host-http.execute：受控 HTTP 请求；显式 sink、timeout、credential reference 与 generatedHeaders。无 guest socket、系统代理、自动重试或自动跟随重定向。
- host-http-stream.open、read、close：三个流端口。按需拉取原始 bytes，不先缓冲整条响应。SSE/NDJSON 的外部格式解释由 adapter 完成。
- host-media.info、read、create、write、finish、release：六个媒体端口。read 分块，create/write 为受管 staging，finish 根据真实内容确定格式，release 清理租约/未完成输出。
- host-credential.capabilities、sign：两个凭据端口。只能得到获授权模式或签名结果，不能取得原始密钥。specJson 使用公共 signSpec 闭集，不允许任意脚本签名器。
- host-state.get、put：两个受限状态端口。容量、包/绑定/请求归属由 host 验证；它不是生产数据库访问，也不代替 durable session checkpoint。
- host-log.log：一个诊断端口。明文密钥、签名、HTTP 头值/正文/敏感 URL 不进入日志；结构化诊断与普通用户文案分层。
- host-stream.emit-event：一个有序事件端口。只有协商 stream-events-v1 后可用，消费失败不能丢弃后继续假成功。
- host-checkpoint.read、write：两个持久快照端口。write 使用 expectedRevision CAS，不能改冻结包/模型/操作/输入锚点，不能写秘密或长期保存临时 handle。

基础导入与可选特性以[host-feature-catalog](protocol/host-feature-catalog.json)协商；未知 required feature 前置拒绝，optional 缺失不能被解释为已支持。stream-events-v1 同时要求受管 HTTP 流读取与事件发送。资源政策来自 [schema-policy](protocol/schema-policy.json)，不能手写第二份默认值；这些是解析/VM/传输安全预算，不是模型数、业务操作数或恢复次数限制。

## 3 显式开发配置

CLI development.json 的完整闭集样例见[作者指南](AUTHORING.zh-CN.md)。Rust 声明见 [workspace::Config](crates/tools/src/workspace.rs)。路径参数必须为绝对路径；输出位于显式、带标记的 development workspace/receipts/*.json。作者工程、运行包与执行 workspace 是三个不同目录。

configuration 必须同时与绑定和每个 Invocation.payloadJson 内的 configuration 一致。bindingId/profileId 同值；配置修改要形成明确的新绑定版本，不让旧异步任务静默改配置。基础 echo 的 supportedFeatures=[] 是刻意选择；要启用 stream-events-v1，必须由真实支持该特性的宿主显式列入 config.trust.supportedFeatures，并在 Invocation.enabledFeatures 使用实际协商结果，而非盲拷全部 feature 名字。

### 网络授权

每个允许 origin 写完整 scheme://host:port，使用实际启动端口。例如开发服务随机分配 127.0.0.1:端口后，从 ready 消息读取 origin，再显式写入 config.network.origins。不能用 *、默认端口猜测或服务名字授予地址权限。

私网/loopback 请求需三方共同成立：manifest 的 endpointPolicy 含 allowPrivateNetwork、requestedCapabilities.allowPrivateNetwork=true，以及开发 grant 的 allowPrivate=true；还须满足所选 staticHosts/userConfigurable 策略和精确 origin 授权。只写其中一个不生效。denylistedHosts、deniedHosts、deniedNetworks 继续生效。link-local、未指定、组播和保留地址不因 allowPrivate 而开放。

DNS 所有答案先校验，再固定到实际连接；重定向不自动跟随，后续 URL 仍需同一权限门。不要向 guest 提供自建 HTTP 客户端绕过这些检查。host 只理解 URL/权限，不按厂商/模型/节点特判。

### 非秘密凭据 reference 与显式 secretFile

配置的 credentials 每项完整形状如下，替换为实际获授权的本地绝对文件；不要把真实 key 写进此 JSON 或本文。

~~~json
{
  "reference": "development-service-reference",
  "secretFile": "C:/sdk-private/service-key.bin",
  "publicId": null,
  "customHeader": null,
  "modes": ["bearer"],
  "origins": ["http://127.0.0.1:12345"],
  "ttlMs": 60000
}
~~~

这只是一项 credentials 条目，不能代替完整 Config。非秘密 configuration 里放该 reference，例如 credentialRef；profile.credentialSlot 指向 /credentialRef。custom-header 模式须显式填写 customHeader，并与 generatedHeaders 中的名称一致。原 key 只由显式 secretFile 读取进入 vault，按请求权限注入；不从用户设置、环境、默认安装文件或浏览器存储读。sign 只接受实际授权的公共算法/模式；当前不支持的 http-sigv4、oauth2-device-code 等模式明确拒绝，不推断协议。

Config.trust.publishers 是显式可信发布者列表，每项字段 publisherId/keyId/publicKey/pluginPrefixes；publicKey 是完整 minisign 公钥 box 文本，不是私钥也不是公钥文件路径。revokedPlugins、revokedVersions、revokedKeys、denylistedHosts、isFresh 与 supportedFeatures 也必须显式提供。可信列表由宿主/开发者批准，不能采信包自带的信任声明。check/pack 命令的四个 trust flags 是另一个明确入口，六操作只使用 config.trust。

### 媒体与 ffprobe

Config.media 每项为 id、实际绝对 path、kind，用于把公开 media 输入绑定到本次导入。CLI 只替换 handle，不替作者猜 MIME、长度或几何值；无关/未用素材 provisioning 会被拒绝。guest 不能得到源路径，也不能从任意文件名读主机文件。

图像用真实字节探测尺寸/格式；音视频需要显式 probe 配置，例如：

~~~json
{
  "executable": "C:/sdk-tools/ffprobe.exe",
  "timeoutMs": 5000,
  "frameTimestamps": true
}
~~~

executable 必须是实际 ffprobe 的绝对路径；不会从 PATH/生产安装目录查找或下载。缺工具、失败输出、缺少请求的实际时间戳必须明确失败。VFR 以 ffprobe 的真实时间戳处理，不用 duration×fps 推算；mock probe JSON 只能证明 probe 协议分支，不能充当实际音视频验收。

公共几何是源像素，时间是毫秒半开区间。检查媒体 ID 闭集、范围、遮罩尺寸和同包/绑定上传来源后才产生作用；不裁切用户输入来过校验。重启恢复保存的是受管内容身份，重建新 handle；不能重用已过期、其他请求或旧进程的租约。输出扩展名/MIME/byteLength 取真实内容，不信 Content-Type 或远端 URL 自报。

## 4 事件 ACK、预算、取消与恢复

EventSink::consume 成功返回才确认消费。参考 CLI 将事件写入 events.jsonl 并 sync_all 后 ACK；写盘失败、丢弃消费、乱序/重复/串 requestId、超预算都失败。sequence 从 1 严格递增；进度和文本/媒体片段不是终态，最终仍需唯一 Response。worker EOF、HTTP EOF、部分产物或连接关闭不能自动当成功。

真实 HTTP 流 read 按显式 maxBytes 拉取；慢消费者受总缓冲预算约束，关闭/取消/读取失败释放连接，不返回假 EOF、不重连生成。媒体 chunk 仍需要活的归属租约。usage 是真实 reported 累计快照，不按重复事件/查询次数相加；没有报告则 null，不能估算或补默认用量。

Control 的父取消令牌须贯穿 ManagedHost/事件消费/清理，不能重建一个不相干 token。HTTP plan timeout、请求 deadline、VM fuel/memory/wall 和输出字节预算各有边界，超限显式失败，不静默截断。WASM 的 guest 活跃执行预算在 host RPC 等待期间暂停计时；宿主请求 deadline 仍可覆盖等待。profile 是本机安全解释器，采用不晚于父 deadline、manifest.wallMs 与 Invocation.budgetMs 的派生截止时间，凭据等待/映射/响应编码也在该范围内；不会宣称它运行在 WASM 沙箱。

共享 Session 在 submit 前持久记录 prepared/acceptance_unknown 锚点。一旦得到有效 accepted 的真实 vendorTaskId，共享 finish 必须在外部 Response 交付前原子保存；profile 不需要第二套任务表。query/cancel 从同一 checkpoint 恢复固定包版本/hash、schema、配置引用、真实远端 ID 和 resumeToken。包升级或冻结输入不匹配明确失败，不换新版本继续跑旧任务。

若远端可能已经受理而回执丢失，保持 ACCEPTANCE_UNKNOWN。不能生成任务号、把异常改成失败后自动 submit，或声称没有远端保证的 exactly-once。只有现有真实远端幂等/查询证据才可找回原任务；无证据就明确不可恢复，不能猜。关闭 CLI/页面只停止本地等待/订阅，不自动 cancel。

取消结果严格区分 remote_requested、remote_confirmed、local_only、unsupported。remote_requested 仍须 query 到真实状态；只有 remote_confirmed 才能确认远端取消。两个任务只取消指定任务；禁止共享全局 interrupt 或超时暗重试。

## 5 严格输入、表单和 Profile

SchemaRegistry 注册先验 digest/本地引用/递归与关键词策略，再用同一原生 JSON Schema 引擎执行。必填缺失、额外字段、范围/枚举错误都拒绝，不在正式提交补默认值、删除未知字段或修复 LLM 文本。namespace、operation/version/schemaDigest、配置、媒体与 required feature 全部先验；input 与 result 按传输 operation 选精确分支。

[参考表单](tools/form-reference/index.html)只导入本地公开 capabilities、完整 request seed 与非秘密配置，导出精确 UTF-8 Invocation 和摘要，不连接模型/服务，也不创建媒体租约。构建要求 tools/form-reference/package.json 指定的 Node >=25；npm ci 后 npm run build。浏览器展示不代替 Rust 验证，unsupportedInteraction 必须阻断导出可执行外观的请求；禁止用 JSON 文本框替代缺失的必填交互。导出的同一文件还需真实 CLI 执行和媒体验真。

Profile 支持范围见[profile-semantics](protocol/profile-semantics.json)及[作者指南](AUTHORING.zh-CN.md)。目标仅 bodyTemplate/resultTemplate；映射见证先物化再复用公共 operation 语义验证，远端 JSON 存在/类型只在运行时保证。静态不支持 streaming、binary upload/provenance、租约媒体输出或复杂签名/条件工作流时安装即拒绝，不能在运行失败后回退 Rust 或走私有宿主端点。

## 6 验收与候选状态

可以复用公开 tests/fixtures，但每次结论必须绑定真实执行命令、退出码、包/源/工具摘要、请求、回执与故障证据。MOCK 与 LIVE 分栏；record/replay 的离线端口匹配不是实时服务可用证明。conform 中 EVIDENCE_DECLARATION 仍只是声明，不能替代实际矩阵。

- G00：主程序/v1/用户数据隔离、独立 workspace 和失败检查。
- G01：公开合同、生成物与输入/结果/权限/能力校验闭合。
- G02：WASM/worker、十八端口、媒体/HTTP流、事件、恢复/取消实际执行。
- G03：模板、作者 CLI、Profile 与参考表单工作链。
- G04：三类独立模拟服务、真实样例和正常/故障矩阵；范围标 MOCK。
- G05：自包含候选导出、阻断主仓访问后的构建/运行、隔离作者新增操作/参数。
- G06：实际候选复现、来源/依赖/许可证、最终隔离与自审；这些闭合后才可 SDK_READY。

以上是门的含义，不是本文替门宣告 PASS。SD28 的真实正负自测/P4子矩阵可以有局部通过结果，C35/C36 仍保持 required=true/NOT_RUN，完整矩阵缺证据必须非零。后续导出、隔离作者及可复现证据到位后再跑全量；不能把后续所需材料伪造为先决已通过。候选流程先产真实复现/清单证据、验证 G00–G05，再完成 G06/SDK_READY，不能循环要求尚未产生的最终状态。

export_sdk.py、check_author_evidence.py、verify_release.py 的候选发行工作链在本稿时点尚未完成验收，未来命令名称不等于已有可用功能。当前不声明发行 READY、全矩阵全绿或任意真实厂商 LIVE 通过。SDK 完成后仍须各生产切换单元单独接入 GUI/Creator/F1/MCP 共用内核并通过 G07；所有 modules 保持 NOT_ADOPTED，既有产品路径不会因参考宿主测试而切换。

LicenseRef-Proprietary 是当前许可标识，本文没有授予额外使用或分发权。正式候选还需完整的许可证/依赖来源清单；独立构建、签名或验收都不能代替权利人的分发授权。
