# v2 可执行合同决定（SD02）

公开字段以 schemas 与 protocol 为源。此文件记录本次物化的决定，不定义第二套 DTO。当前安装版已支持工作流关联的v2包导入；范围与未验事项见 [预览版说明](DEVELOPER-PREVIEW.zh-CN.md)，不据此声明所有业务模块接入。

- 现有 11 份设计 schema 与 operation-contracts.json 保持原字节。绝对 URI 是随包资源身份，不允许联网解析。
- manifest 保留 v1 身份、发布者、入口、权限、资源及 minisign-prehashed 语义；版本范围仅 [2,2]。全部字段显式，无 protocolVersion 旧别名。动态能力只由 runtimeCaps 返回，manifest 不复制能力参数。publisherId/keyId 为明确 null 的未签名 local 包，签名时二者必须同时存在，身份需由显式信任输入核验。
- 同一个 host-ports schema 的 $defs 与 x-wit-functions 产生 WIT 与 native DTO。没有平台名、数据库、界面或业务宿主入口。worker 帧仅包装一次调用、关联号、端口和唯一回执。
- JSON 字符串中的 u64 沿现有媒体 byteLength 的十进制表达，避免 JS 精度损失；WIT 转整数必须检查溢出。JSON 普通整数向 WIT u32/u64 同样检查，不能截断。
- HTTP 流 read 显式 eof，错误通过 hostError；close 释放连接。同步 execute 与流 open 使用同一请求/凭据和出网授权。stream-events-v1 同时要求流读取与事件端口。
- media-info 由宿主返回源像素、时长与可选实际时间戳。release 为租约释放；checkpoint 只保存内容身份，不持久化临时句柄。
- reference checkpoint 的 expectedRevision 提供显式 CAS；revision 使用十进制字符串。状态是开发证据，不是生产 ai_tasks 的替代。
- profile 仅支持封闭的 JSON pointer 映射和常量模板，无 eval、脚本或任意组件。复杂协议由代码 adapter 负责。未实现语言/鉴权声明需明确拒绝，不以语法可解析宣称运行支持。
- std 最初未发布可执行 profile；2026-10-26 S0 发布 speech-v1 的 synthesize/batch/voices.list 及固定 input/output/resume 摘要。注册不代表所有包实现这些操作，namespaced generic 操作仍须完整实现。
- 资源政策在 protocol/schema-policy.json 统一发布，数值仅为解析/传输安全预算。可由宿主明确协商更小的预算，但不能静默裁切用户业务输入；超限显式失败。不是模型数、业务操作数或时长上限。后续更改通过公共合同修订，禁止在各 crate 写独立默认值。
- 完整 JSON Schema 验证使用成熟原生库，SD04 冻结精确版本。先元 schema、关键词/本地 ref/无递归和预算审查，再编译；原生库不认识或不实施的约束不得忽略。format 必须是已支持且实施断言的格式。
- 规范覆盖：身份/签名/权限→manifest；工作进程→worker-envelope；宿主/HTTP流/媒体/凭据/事件/快照→host-ports；持久恢复→checkpoint；配置型适配→profile；预算/方言→policy + schema-policy。业务输入/输出继续由各 capability 的 schema 引用及统一语义验证器约束。

尚未执行运行时验收；此步局部校核不等于 SDK_READY。

SD08 实施前补充：usage 的公共 metric/unit 名称在 protocol/vendor-adapter-protocol.json 的 usageNames 中登记；未登记的计量名、货币或单位使用包发布者 namespace/name 原样报告，不估算、不换算、不限制扩展集合。列表不是全部世界货币或全部未来计量的闭集。支持的标准 capabilityKey 同样只从协议表读取，禁止验证器另写名单。

SD11 实施前补充：workerFraming 冻结 u32 小端长度 + UTF-8 JSON，EOF 不能代替终态。单一编解码放原生 ports，worker/transport 共用；它不是新业务协议。公开资源 policy 补齐模块字节、表元素和实例/内存/表数量预算，覆盖代码包与工作流包的沙箱资源；memoryBytes 按整个 Store 累计而不是每个 memory 分别放大。数量只限制 VM 资源，不限制业务操作或并发请求。budgetMs 计 guest 活跃执行，host RPC 等待暂停计时；取消可中断 host 等待，禁止把远端等待误当 guest CPU 超时。

SD14 合同补齐：`host-ports.schema.json#/$defs/signSpec` 定义 version=1 的封闭签名参数，`specJson` 必须先经该 schema 校验。HMAC-SHA256 显式选择 hex/base64url；HS256 JWT 的 `claimsJson` 必须是 JSON object，头由公共 protocol 固定；HMAC credential 的 presentation 同样从 protocol 读取。guest 只能得到签名，bearer/custom-header 原始密钥只由 HTTP broker 注入。

`protocol/vendor-adapter-protocol.json.security` 是受保护请求头、IP 网段、生成头和签名语义的唯一源。生成头首版支持 bearer/custom-header，targetField 与 name 表示同一 HTTP 头；没有隐式正文键注入。guest 明确请求且开发者明确授权的源才可出网，所有 DNS 答案校验后固定给实际连接；不使用系统代理，不自动跟随重定向、不自动重试。后续请求仍必须通过同一门。timeoutMs=0 表示没有此 plan 的单独截止时间，宿主请求的取消/截止时间仍生效。private HTTP 需 manifest 的 endpointPolicy 与布尔能力及开发授权共同成立；link-local/未指定/组播/保留地址始终拒绝。凭据、媒体与权限均绑定包/绑定/请求，权限撤销可中断等待中的 HTTP。

HTTP 媒体下载按完成后的字节探测种类、MIME 和扩展名；不信任响应 Content-Type。multipart 只接受已测媒体 MIME 与规范扩展名，HTTP 栈生成 boundary，禁止用用户 header 偷换。媒体和 multipart 上传都流式，不把原文件整体读进内存。基础网络诊断只记录 method/scheme/host/port/status/outcome。S0 另提供 before-send 持久化：记录脱敏 URL、实际正文文字及媒体摘要，排除凭据头和 broker 已知秘密；是否启用直接库钩子由嵌入宿主配置，CLI 默认启用本地持久化。所有运行仍只属于独立 SDK 开发宿主，主程序尚不能导入 v2。

SD15 流读取语义：stream open 只接受 `sink.kind=buffer`，正数 `sink.maxBytes` 限制待消费的 native chunk，不限制完整响应长度，且不得超过公开 maxBufferedHttpBytes。单次 read.maxBytes 须为正且不超过 maxHttpChunkBytes；同一请求全部流的保留缓冲合计不得超过 maxBufferedHttpBytes。按需拉取，无后台预读；收到 EOF 后可重复读 EOF，直至显式 close。错误、取消或丢弃读取 future 会关闭并移除该流，不得返回假 EOF，也不得自动重连或重新提交。SSE/NDJSON 的内容解释完全由 adapter 处理。
