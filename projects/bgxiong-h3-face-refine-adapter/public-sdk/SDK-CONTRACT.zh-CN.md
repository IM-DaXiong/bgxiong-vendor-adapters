# SDK 接口规范

合同版本2.0.0。开发者预览版已提供公开Rust SDK、独立CLI/worker、模板、schema和真实ComfyUI样例；SDK完整候选治理仍未完成，产品可用范围见 [预览版说明](DEVELOPER-PREVIEW.zh-CN.md)。作者接口以随包 sdk/rust/ 实际导出和同版本schema/WIT为准，不能使用v1 crate冒充v2。

## 1 复用基础和明确增量

现有基础：Invocation/Response；capabilities/probe/upload/submit/query/cancel；manifest/RCD；host-http/media/credential/state/log；worker沙箱；包安装与版本pin；SubmitResult/QueryResult/TerminalFailure；媒体句柄与开发CLI。保留这些职责和执行内核。

新增或收口：动态能力/操作/schema/UI描述；通用配置schema；统一严格业务payload；多模态媒体role校验；结构化result/usage/resumeToken；可选有序流式事件；公开包自包含；跨官方/云端/工作流的同一套conformance。现行 pluginParameters 的“只要JSON合法”不能作为新标准验证器。

## 2 对外发布包

发布根包含 README、OPEN-STANDARD、SDK-CONTRACT、CONFORMANCE、protocol/、schemas/、sdk/rust/、wit/、templates/、examples/、fixtures/、tools/ 和带摘要的 release-lock.json。这些源码和工具目录已包含在预览安装资料中；release-lock.json保留独立完整候选的未封存状态，不是全量发行验收通过证明。运行工具位于bin/，外部工具链及联网依赖准备见作者指南。

独立适配包至少含 manifest、WASM或可构建源码、完整公共SDK/模板依赖，以及嵌入代码的能力和schema资源。业务资产可独立版本化，但运行时不得回作者源码目录读取。workflow-map 等是平台包内部资产，不进入通用 manifest 的平台字段。

runtime-caps v2 顶层固定为 schemaVersion=2、pluginId、pluginVersion、capabilityContracts；transportSlot由每项声明提供，取消旧slots中重复的参数约束。未落入既有领域槽的能力使用 generic；其通用表单、提交、结果与MCP入口必须同批可用，不能只允许注册不能执行。manifest 沿现行包身份、权限、入口、requiredFeatures/optionalFeatures 升级 schemaVersion，不增加“官方专用”或“Comfy专用”类型。namespace 按发布者身份验证。

## 3 声明结构

CapabilityContractV2 的精确结构以 schemas/capability-contract.schema.json 为准。schemaVersion=2；schemas[] 包含每项 id/digest/schemaJson；schemaJson 是完整JSON schema文件解码得到的字符串。digest 对该字符串原样UTF-8字节做SHA-256（无BOM，不重排对象、不改空白、不做Unicode归一化），避免不同语言重序列化数字导致摘要漂移。schemaJson解析后做元schema校验，发布包另校验资产字节SHA。

每项 operation 的 schema 引用必须落入 schemas[]，不得使用网络 $ref。input/output/resume/config schemaJson 解析结果 必须过元schema检查及闭集对象检查；uiSchema为内联控件描述，按ui-schema.schema.json校验，字段路径与控件类型的匹配作为 ui_diagnostics，不以缺控件阻断合法请求；它不是另一份JSON schema，也不通过任意文本推测控件。媒体槽 inputPointer 指向 input 里的 {mediaId} 或其数组；mediaId 必须落在本次 media[] 闭集，种类/数量相符。图像/音频/视频/文件共用句柄机制；新媒体种类须公共版本扩展。

标准操作 schema 由公共包提供，第三方实现同名标准操作必须引用相同摘要。平台特别参数放该包的 namespaced operation/input schema，不能改变 std 操作原义。对外标准定义参数的表达/校验方式，不硬编码某家所有参数。

## 4 Wire 输入与投影

Invocation 延续字段 protocolVersion、operation、profileId、capabilitySlot、requestId、budgetMs、payloadJson、bindingId、negotiatedVersion、enabledFeatures；v2正常调用全部显式填写，版本固定2，未启用features为空数组，不能缺字段补默认。payloadJson依然是JSON字符串；Response互斥dataJson/error。

submit payload 的字段固定：schemaVersion=2、modelId、capabilityKey、operationId、operationVersion、inputSchemaDigest、input、media、uploads、configuration。input 对所选schema严格校验；media是MediaInputV2数组；configuration只含已验证的非秘密配置，凭据通过broker获得。没有host/storyId/nodeId/trackId、writeMode、旧provider extra或第二个model别名。

query/cancel payload 固定：schemaVersion=2、modelId、capabilityKey、operationId、operationVersion、vendorTaskId、resumeToken、configuration。resumeToken按冻结resumeSchema验证，允许null仅当schema明确允许。query重建独立worker仍可执行；不能依赖submit进程内存或先前临时handle。

capabilities payload={}，返回严格RCD和全部内嵌声明；零网络。probe的类型化输入区分package_validation与connectivity；前者零网络，后者只能用显式授权配置，不把可连通当模型/节点就绪。upload仅收媒体句柄和声明上传用途；其远端引用留adapter，不让用户填vendor文件ID替代媒体引用。

应用层请求可以包含结果写回host和数据库版本；统一业务内核解析后只将上面的中立payload交给adapter。GUI与MCP的输入投影共用同一个函数；SDK作者不实现节点版本CAS，不打开工作台也可以测试。

## 5 作者代码接口

保留一个正式入口 `invoke(Invocation, &mut dyn Host) -> Response`，不新增每平台入口。

SDK导出类型：Invocation、Response、CapabilityContractV2、OperationSubmitV2、OperationTaskRefV2、MediaInputV2、SubmitResult、QueryResult、OperationResultV2、Output、TerminalFailure、UsageV2、StreamEventV1。OperationResultV2是result.schema的联合，必须按operation-contracts.json选择对应分支，不得仅验证联合后接受错误分支。跨语言类型只从schema生成；Rust SDK必须可native test及WASM，不依赖Tauri或私有主crate。

校验接口概览（精确签名以同版本sdk/rust/实际导出为准）：

```rust
fn validate_contract(contract: &CapabilityContractV2) -> Result<(), ContractError>;
fn validate_input(contract: &CapabilityContractV2, request: &OperationSubmitV2)
    -> Result<ValidatedOperationInput, ContractError>;
fn validate_result(context: &ValidatedOperationInput, result: &OperationResultV2)
    -> Result<(), ContractError>;
```

ValidatedOperationInput仅校验器可构造；host与native CLI采用同一完整验证实现，guest使用同源生成类型及关键字段检查。原生完整JSON Schema引擎不强制编入no_std的guest，WASM和native的基础作者接口均须可构建。校验器不做LLM语义判断，不换模型/改参数。

Host复用已有受管HTTP、媒体读写/上传下载、凭据操作、受限状态和日志接口。新增 `emit_event(StreamEventV1) -> Result<(), HostError>` 仅在协商stream-events-v1时可用；同版本WIT已提供host-stream受管导入，精确参数以wit/vendor-adapter.wit为准；这不表示当前工作流产品界面已验所有流式场景。不得把客户端的HTTP连接传给guest，不让guest自己开socket。随包包含独立参考host、固定v2 worker、CLI及WIT；当前应用消费范围另见预览说明。需要新模块接入时按固定版本核对，不直接覆盖生产worker。本次预览资料收尾没有修改WIT。

prepare是SDK本地纯校验/平台组包阶段，不是第七个远端操作；不创建任务、不上传素材。preview在标准中是显式声明的业务operation，以submit/query同生命周期执行，不复用正式成片假装预览。

## 6 回执、结果与事件

SubmitResult沿accepted/completed判别：accepted必须真实vendorTaskId；completed的data必须符合outputSchema；outputs允许空数组，例如结构化分类、检索或校验结果。是否必须有媒体/文字输出由具体操作合同决定，公共层不强制生成媒体。QueryResult沿现有任务状态，终态失败必须TerminalFailure；不创建另一个宿主状态枚举。标准增加的data、usage和resumeToken字段一次性在v2 schema定义，禁止某个adapter先私塞字段。

UsageV2={measurements:[{metric,unit,amount,source:"reported"}]}；amount为非负十进制字符串，不用浮点金额；metric/unit使用公共命名或发布者namespace扩展，不能靠主程序认识厂商字段；无报告则usage=null。媒体outputs的MIME/字节/时长必须真实，多个输出按声明角色返回。appliedParams只报告写后读回或服务确认过的参数。

StreamEventV1={requestId,sequence,type,payload}；sequence从1严格递增；type=progress|text_delta|media_chunk|usage；payload按对应闭集schema（见事件schema）验证。事件的requestId由host核对，不允许串任务。最终Response是唯一终态；EOF或worker退出不是成功。media_chunk的handle仍受租约和落盘规则，usage事件为累计快照，不在每次事件重复累加。背压由host控制，emit_event失败不能丢事件继续假成功。

resumeToken是包的受schema校验不透明数据，不含秘密；主程序只保存并回传，不解释vendor格式。新查询返回更新token时用既有任务checkpoint原子保存。流式是否可续由声明和远端证据决定；不能把序号本身当远端续传保证。

## 7 明确的扩展限制

新增既有原语可表达的参数/能力操作，无需更新主程序；新增任意JS控件、设备访问权限、未支持鉴权或push webhook接入不自动获得执行权。需要回调而不能query的API须先有公共callback特性及宿主实现，否则显式不支持。SDK全面性靠可发现边界与公共扩展规则，不能用未实现feature冒充全平台可用。

核心边界、字段和行为在本次定稿；WIT、manifest/RCD 生成物、SDK 类型、CLI、模板和运行测试由独立 SDK 实施计划逐文件交付，必须符合本规范；当前完整候选验收状态见 [RELEASE-STATUS.json](RELEASE-STATUS.json)。正文及schema不能由后续工作台计划重新决定。设计检查只验证已提供的设计资产，不表示发布物齐全或运行验证通过。


## 8 跨字段验证与宿主接口约束

以下为规范要求，schema之外由统一验证器完成；设计检查器只覆盖明确列出的fixtures，不能冒充完整验证器：

- invocation的operation决定payload与result分支；submit不能返回query结果。profileId必须等于bindingId；无绑定的capabilities/package_validation由CLI提供隔离临时绑定身份，不授予网络/凭据权限。协议级错误code只用response.schema闭集；retryable不代表允许自动重提可能已受理任务。
- modelId、capabilityKey、operationId/version、inputSchemaDigest必须全部匹配已冻结声明；configuration按configurationSchema验证。异步必须可查询，resumable=true必须有resumeSchema；无resumeSchema时resumeToken只能null。同步声明不得返回accepted；异步可直接完成。
- operation outputSchema只验证data；输出数组的角色、媒体种类及数量由该操作的outputSlots声明；其它格式约束由公共profile定义，包不能隐式省略。appliedParams非null时只能含input中存在且符合原字段schema的已证实值；与冻结输入有差异必须显式展示。
- capabilities零网络；probe分package_validation和connectivity。package_validation仅校验包、schema和配置结构；connectivity检查配置授权后才允许网络，结果分别记录连接、认证、资源就绪，不能一个布尔值表示全部就绪。upload由所选操作的媒体槽驱动，上传回执只能作为同包、同绑定的受管引用使用；禁止跨包重用远端引用。
- cancel返回结果区分remote_requested、remote_confirmed、local_only、unsupported；只有remote_confirmed可确认远端取消，remote_requested继续query到真实终态。权限/网络错误走Response.error。关闭UI只解除订阅；禁止默认调用cancel。
- schema注册必须拒绝重复id、缺失引用、摘要不匹配、跨包未授权引用、递归引用；禁止加载网络资源。字段指针按JSON Pointer解释；repeat内以items描述单项，不用路径猜业务。UI必须能覆盖每个用户可写的必填字段；不支持的schema或交互返回明确原因。
- media.id唯一；所有引用必须存在且符合角色、种类和min/max；maxCount=null表示未声明上限，仍受公开资源预算约束，不能静默裁切。byteLength须可解析为u64；fileName为basename，拒绝路径、点目录与控制字符。媒体句柄由host验证归属、租约、MIME与字节信息，不能信任guest自报。
- time_span值为{mediaId,startMs,endMs}，0≤startMs<endMs≤源时长；point为{mediaId,x,y}，box为{mediaId,x,y,width,height}，范围在源像素内；mask为{mediaId,maskMediaId}，遮罩须与源尺寸对齐。VFR的时间轴以实际时间戳为准，禁止duration×fps猜帧。以上字段形状须由公共schema生成，平台转换发生在adapter。
- requestId是一次invoke身份，主机重复派发必须保持可追踪关系。remoteIdempotency=true时adapter将其映射为真实远端幂等键；没有远端保证不能宣称exactly-once。主机对不确定受理显式记录，不自动再submit。
- Host端口的既有HTTP/媒体/凭据/权限/日志语义继续复用；新增流式端口须包括受管HTTP流读取（SSE/NDJSON可由adapter解析）与事件发送，不能只有事件接口而先缓冲完整响应。端口限制由能力协商公开，未支持的流式上游显式拒绝。

## 9 设计文件与请求选择

operation-contracts.json是六种传输操作的schema选择表；control-operations.schema.json定义capabilities/RCD、probe、upload及cancel回执。interaction-values.schema.json是几何/时间控件值的公共源。不得因顶层union恰好匹配而接受错误操作回执。

uploads是submit中的必填数组，未提前上传时为[]；每项{mediaId,uploadRef}只能来自同包/同版本/同绑定下成功upload的回执，mediaId必须引用本次media数组且不可重复。host保管来源并校验归属，adapter解释opaque uploadRef，用户不能自行填远端ID。未提前上传由submit按同一媒体句柄上传；无上传缓存不等于无素材，不允许跨配置复用引用。uploads只为优化上传，不改变业务输入或结果。

package_validation结果必须connection/authentication/resources全部not_checked；connectivity允许逐项标ready/failed/not_checked，失败须有issues，不能把未检查当就绪。configuration在probe时按选中包各能力的配置schema验证；多种配置合同不同时分能力建立绑定，不混成无校验字典。

outputSlots中role唯一；输出每项必须匹配声明role/kind及数量；媒体kind=media时进一步核对media.kind。未声明的输出拒绝，不能只取首项。数据型能力允许outputSlots=[]且outputs=[]，其data仍必须严格过outputSchema。部分输出按role+数组内序号稳定保序，同一任务不得挪用序号替换已交付产物；host复用既有部分结果对账机制。
