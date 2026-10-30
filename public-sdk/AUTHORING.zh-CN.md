# 独立适配包制作指南

本指南对应合同2.0.0的可执行开发者预览版。安装目录已带完整SDK、CLI/worker及ComfyUI作者样例；支持的图片生成、视频生成与编辑入口和未验边界统一见 [预览版说明](DEVELOPER-PREVIEW.zh-CN.md)。独立完整SDK的G00—G06与各产品入口验收分开记录，不能据单一样例晋升SDK_READY。此处提供通用SDK命令与合同；用户自己的ComfyUI工作流从安装目录 user-examples/comfy-workflow-authoring-v2/README.zh-CN.md 开始。

字段与语义以[开放标准](OPEN-STANDARD.zh-CN.md)、[SDK 合同及作者接口](SDK-CONTRACT.zh-CN.md)、[schema](schemas/)和[六操作分派表](operation-contracts.json)为准。可调用的 Rust API 见 [SDK 导出](sdk/rust/src/lib.rs)、[Adapter 入口](sdk/rust/src/guest.rs)、[Host 接口](sdk/rust/src/host.rs)和[生成 WIT](wit/vendor-adapter.wit)。示例不另定义字段。

## 1 准备独立工具和目录

需要 rust-toolchain.toml 锁定的 Rust 1.95.0、wasm32-wasip2 target，以及本机对应的原生链接工具链。即使目标是 WASM，proc macro/build script 仍需原生工具链；缺少工具链明确失败，不能指定开发者机器上的链接器路径作兜底。

以下示例使用 PowerShell 7。先将两个示例目录替换为本机实际绝对路径；CLI 不依赖 PowerShell，其他 shell 可传递相同参数。SDK 必须是完整、独立的公开树，作者目录不能是主程序或用户数据目录。

~~~powershell
$Sdk = (Resolve-Path 'C:/sdk/vendor-adapter-v2').Path
$Root = 'C:/sdk-work/echo-demo'
New-Item -ItemType Directory -Path $Root | Out-Null
$Project = Join-Path $Root 'author'       # 必须尚不存在，init 不覆盖空目录
$Workspace = Join-Path $Root 'workspace' # 由下面首次 caps 显式创建
$InputDir = Join-Path $Root 'inputs'
New-Item -ItemType Directory -Path $InputDir | Out-Null
$Suffix = if ($IsWindows) { '.exe' } else { '' }

rustup toolchain install 1.95.0 --profile minimal --target wasm32-wasip2
$env:CARGO_HOME = Join-Path $Root 'sdk-cargo-home'
$env:CARGO_TARGET_DIR = Join-Path $Sdk 'target'
cargo fetch --manifest-path (Join-Path $Sdk 'Cargo.toml') --locked
cargo build --manifest-path (Join-Path $Sdk 'Cargo.toml') --locked --offline -p bgx-adapter-v2-tools -p bgx-adapter-v2-runtime --bins
if ($LASTEXITCODE -ne 0) { throw 'SDK tools build failed' }
$Cli = Join-Path $Sdk "target/debug/bgx-adapter-v2$Suffix"
$Worker = Join-Path $Sdk "target/debug/bgx-adapter-v2-worker$Suffix"
& $Cli --help
~~~

首次安装工具链/fetch 是显式联网步骤。之后的 --offline 只证明该锁定缓存足够，不保证另一台机器已有缓存。工具和 worker 路径均显式传入；不能覆盖或发现主程序安装的 worker。

## 2 Rust：init、build、check、pack

~~~powershell
& $Cli init --sdk $Sdk --dir $Project
if ($LASTEXITCODE -ne 0) { throw 'init failed' }
& $Cli build --sdk $Sdk --project $Project
if ($LASTEXITCODE -ne 0) { throw 'explicit online author build failed' }
& $Cli build --sdk $Sdk --project $Project --offline
if ($LASTEXITCODE -ne 0) { throw 'locked cached author build failed' }
& $Cli check --sdk $Sdk --worker $Worker --project $Project
if ($LASTEXITCODE -ne 0) { throw 'package check failed' }
$Package = Join-Path $Root 'runtime-package'
& $Cli pack --sdk $Sdk --project $Project --out $Package
if ($LASTEXITCODE -ne 0) { throw 'pack failed' }
& $Cli check --sdk $Sdk --worker $Worker --project $Package
if ($LASTEXITCODE -ne 0) { throw 'precompiled package check failed' }
~~~

作者 build 使用工程自己的 .bgx-sdk/cargo-home、target、cargo-run 和 logs，与上节的 SDK 编译缓存分开。首次空作者缓存直接 --offline 应失败；必须先显式联网构建。所有构建坚持 --locked；缺依赖不自动更新锁。Cargo.lock 是作者可显式维护的依赖锁，但依赖仍只能来自项目/所选公开 SDK 和锁定公共 registry，不能跨到主仓私有 crate。

可编辑 Cargo.toml、src/lib.rs、manifest.json、README.md 和作者 Cargo.lock。vendor-sdk/ 与 template-lock.json 为受封存的公开 Rust/WIT 镜像，禁止手改。init 先核对模板、原公开源、生成文件与合法路径，再独占创建目标，只复制白名单。build 拒绝祖先 .cargo/config、config.toml 及显式 Cargo home 内配置/凭据文件；只检查是否存在，不读取内容。遇到此错误应换到无隐式配置的独立目录，不让工具覆盖用户配置。

build 从实际 cdylib target 名字定位 WASM，生成真实 artifactSha256 与 manifest.sha256。内部路径 remap 用于排除构建机源码路径，不继承外部 RUSTFLAGS/wrapper/proxy。源码工程和预编译目录使用同一包校验器；check 会实际执行 capabilities 和 package_validation，且不给网络/凭据权限，不能由此宣称目标服务已连通或业务成功。pack 只产生运行所需清单、入口及可选签名，不带源码、target、缓存、密钥或日志；已有 out 一律拒绝覆盖。已签目录不能直接 rebuild，应回未签作者工程重建并明确重签。

Rust 模板是无外部服务的 generic echo。修改插件身份时同步源码与 manifest；修改 schema 时更新 schemaJson 原始 UTF-8 字节的 SHA-256 和相应引用。一个模型可以有多个能力/操作；显示名不作为身份。新增服务参数由包内 schema 和 UI 控件声明，不能要求宿主增加厂商分支。

## 3 无网络 echo 的完整请求

配置是封闭的开发 provisioning 文档，所有字段显式填写。下面没有密钥、媒体、出网授权或隐式默认值。

~~~powershell
function Save-Json {
  param([string]$Path, [object]$Value)
  $Text = $Value | ConvertTo-Json -Depth 100 -Compress
  [IO.File]::WriteAllText($Path, $Text + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}
function New-Invocation {
  param([string]$Operation, [object]$Payload, [string]$Id)
  return @{
    protocolVersion=2; operation=$Operation; profileId='echo-binding'
    capabilitySlot='generic'; requestId=$Id; budgetMs=5000
    payloadJson=($Payload | ConvertTo-Json -Depth 100 -Compress)
    bindingId='echo-binding'; negotiatedVersion=2; enabledFeatures=@()
  }
}
$ConfigPath = Join-Path $Root 'development.json'
$Config = @{
  bindingId='echo-binding'; bindingRevision=1; configurationRef='echo-config-v1'; configuration=@{}
  trust=@{
    publishers=@(); revokedPlugins=@(); revokedVersions=@(); revokedKeys=@()
    denylistedHosts=@(); isFresh=$false; supportedFeatures=@()
  }
  network=@{origins=@(); allowPrivate=$false; allowEmbeddedAuthorization=$false; deniedHosts=@(); deniedNetworks=@()}
  credentials=@(); media=@(); mediaTtlMs=60000
  transport=@{handshakeTimeoutMs=5000; writeTimeoutMs=5000; shutdownTimeoutMs=5000; cancelGraceMs=100}
  probe=$null
}
Save-Json $ConfigPath $Config
$CapsInput = Join-Path $InputDir 'caps.json'
Save-Json $CapsInput (New-Invocation 'capabilities' @{} 'echo-caps-1')
& $Cli caps --sdk $Sdk --worker $Worker --package $Package --workspace $Workspace --config $ConfigPath --input $CapsInput --output (Join-Path $Workspace 'receipts/caps.json') --create-workspace
if ($LASTEXITCODE -ne 0) { throw 'caps failed' }

$ProbeInput = Join-Path $InputDir 'probe.json'
Save-Json $ProbeInput (New-Invocation 'probe' @{schemaVersion=2;kind='package_validation';configuration=@{}} 'echo-probe-1')
& $Cli probe --sdk $Sdk --worker $Worker --package $Package --workspace $Workspace --config $ConfigPath --input $ProbeInput --output (Join-Path $Workspace 'receipts/probe.json')
if ($LASTEXITCODE -ne 0) { throw 'probe failed' }

$CapsResponse = Get-Content -Raw (Join-Path $Workspace 'receipts/caps.json') | ConvertFrom-Json
$Catalog = $CapsResponse.dataJson | ConvertFrom-Json
$Contract = $Catalog.capabilityContracts | Where-Object capabilityKey -eq 'local.sdk.echo/echo'
$Operation = $Contract.operations | Where-Object id -eq 'local.sdk.echo/echo'
$SubmitPayload = @{
  schemaVersion=2; modelId='example-model'; capabilityKey=$Contract.capabilityKey
  operationId=$Operation.id; operationVersion=$Operation.version
  inputSchemaDigest=$Operation.inputSchema.digest; input=@{text='Hello，世界'}
  media=@(); uploads=@(); configuration=@{}
}
$SubmitInput = Join-Path $InputDir 'submit.json'
Save-Json $SubmitInput (New-Invocation 'submit' $SubmitPayload 'echo-submit-1')
& $Cli submit --sdk $Sdk --worker $Worker --package $Package --workspace $Workspace --config $ConfigPath --input $SubmitInput --output (Join-Path $Workspace 'receipts/submit.json')
if ($LASTEXITCODE -ne 0) { throw 'submit failed' }
~~~

这里的 echo 是同步 completed，不支持 upload/query/remote cancel。不要为了演示后面命令而编造任务号。一次 invoke 的 requestId 必须新鲜；query/cancel 使用新的调用 requestId，并用单独 --request-id 指向最初 submit 的 checkpoint。重启命令不加 --create-workspace。

对确实声明上传、异步查询或取消的包，先按[公共 payload](schemas/operation-payload.schema.json)准备对应完整 Invocation，再使用下面的相同命令形式：

~~~powershell
& $Cli upload --sdk $Sdk --worker $Worker --package $Package --workspace $Workspace --config $ConfigPath --input (Join-Path $InputDir 'upload.json') --output (Join-Path $Workspace 'receipts/upload.json')
& $Cli query --sdk $Sdk --worker $Worker --package $Package --workspace $Workspace --config $ConfigPath --input (Join-Path $InputDir 'query.json') --output (Join-Path $Workspace 'receipts/query.json') --request-id 'original-submit-request-id'
# 只有用户明确请求取消时执行；不是关闭页面/CLI的默认动作。
& $Cli cancel --sdk $Sdk --worker $Worker --package $Package --workspace $Workspace --config $ConfigPath --input (Join-Path $InputDir 'cancel.json') --output (Join-Path $Workspace 'receipts/cancel.json') --request-id 'original-submit-request-id'
~~~

这些三个文件不是 echo 示例自动生成的文件。upload 的 media 身份/种类/MIME/文件名/十进制 byteLength 必须与实际文件相符，Config.media 每项为显式 id/path/kind；CLI 仅把非空占位 handle 换成本次受管租约，其余字段仍由共享校验器验真。query/cancel 的 payload 必须逐项匹配原 checkpoint 的 modelId、capabilityKey、operationId/version、vendorTaskId、当前 resumeToken 和 configuration，不能靠重新发现来选择另一个任务。query/cancel 的 Config.media 应为空，由 checkpoint 重建原内容的新租约；不能带本次未使用的旧素材 provisioning。uploadRef 只使用同包/版本/绑定/配置成功 upload 的真实回执。

网络、凭据和 ffprobe provisioning 见[宿主接入指南](HOST-INTEGRATION.zh-CN.md)。运行回执保存在显式 workspace/receipts/*.json；诊断在 workspace/cli/，事件在该调用的 events.jsonl。输入文件原字节摘要与实际执行 payload 摘要分别保留，媒体租约更换不会被伪装成原始输入字节不变。

## 4 Profile：安全解释器的明确子集

language=profile 的入口是 JSON，不是 WASM。它仍走同一 ReferenceHost 的目录、准备、session、结果校验、取消与清理；运行标识为 profile，不伪造 worker PID。它支持 JSON 请求、缓冲 JSON 响应、text 输出和结构化 data；可声明可查询的 JSON 异步受理。它不提供 eval、JavaScript、条件分支或平台专用状态机。

只有 bodyTemplate/resultTemplate 内已存在的目标可映射；null 可显式预留位置，value 是类型见证而非缺字段时的替代值。使用 RFC6901 pointer，拒绝重复/祖先重叠目标；输入/配置来源须能由已声明 schema 静态证明，远端 JSON 是否存在及类型只能运行时检查。request.task 是 query/cancel 的完整任务 payload，response.task 是解析后的 HTTP JSON。URL 只接受静态绝对 HTTP(S)，或必填配置字符串插值 {{configuration:/pointer}}，单遍替换；最终网络门仍由宿主执行。

凭据仅用配置中的非秘密 reference。custom-header 恰有一个 header 的整个 value 等于[公开 semantics](protocol/profile-semantics.json)定义的 {{credential}}；解释器移除字面占位头，再由原 credential/HTTP broker 注入密钥。多个 connectivity probe 只有请求/响应定义完全相同才可共用，否则安装即 UNSUPPORTED_FEATURE。

streaming、binary upload/provenance、复杂签名/工作流分支和需要下载落盘生成租约的媒体输出未在此语法中表达，安装即明确 UNSUPPORTED_FEATURE，并指导作者改用 Rust。不能把远端 URL/任意 handle 冒充媒体租约，不能运行失败后换代码路径。profile 的 record/replay 当前也明确不支持；WASM tape 不能冒充解释器执行证据。

可复制 templates/profile/profile.json 与 manifest.json 到新的作者目录；本地修改后按实际 profile.json 字节更新 artifactSha256，再生成仅列 manifest.json 和 entry 的单空格、相对路径 SHA-256 inventory。随后使用相同 check/pack 命令。build 仅面向 Rust，不会自动为 profile 补字段或生成虚构 WASM。模板静态 URL example.invalid 不是可用服务：要跑 HTTP echo，必须在作者包/配置中明确改成自己的服务及其网络授权。

## 5 签名与显式信任

运行包 inventory 覆盖 manifest.json 和实际 entry；签名文件名来自[公开协议](protocol/vendor-adapter-protocol.json)的 signature，当前为 manifest.sha256.minisig。不要在库存中自造源码文件或忽略未列运行文件。

发布者 manifest 中的 pluginId/publisherId/publisherKeyId 必须真实、匹配自己的 namespace；源码能力 namespace 同步。unsigned local 示例不会获得 trusted 状态。check/pack 的四个 trust 参数必须成组提供，公钥是完整 minisign 公钥 box 文件。包自带的公钥不能自动成为信任来源。

~~~powershell
# 先在未签作者工程中准备合法发布者身份并重新build。
$SignedOut = Join-Path $Root 'signed-runtime-package'
& $Cli pack --sdk $Sdk --project $Project --out $SignedOut --publisher-id 'example.author' --key-id 'release-1' --plugin-prefix 'example.author' --public-key 'C:/sdk-keys/release.pub' --signing-key 'C:/sdk-keys/release.key' --password-file 'C:/sdk-keys/password.txt'
& $Cli check --sdk $Sdk --worker $Worker --project $SignedOut --publisher-id 'example.author' --key-id 'release-1' --plugin-prefix 'example.author' --public-key 'C:/sdk-keys/release.pub'
~~~

上例要求实际存在且获授权的密钥文件，并非 SDK 附带密钥。签名必须明确二选一：--password-file 或 --unencrypted-key；后者只适用于明确选择的未加密测试私钥。不能从环境、默认文件、交互密码或一次失败猜测另一模式。六操作命令的信任由 config.trust 显式提供，不能给 run 命令追加这些 pack/check trust flags。

## 6 record、replay、conform 的实际含义

以下针对本节前面的未改动 Rust echo 运行包；所有记录是 MOCK。record 会真实执行 guest，replay 再执行原 guest 并逐项核对确切端口顺序、参数、响应及最终 Response，离线 replay 不创建真实网络 broker。包版本、inventory、schema 或记录发生漂移均失败。

~~~powershell
$RecordInput = Join-Path $InputDir 'record-caps.json'
Save-Json $RecordInput (New-Invocation 'capabilities' @{} 'echo-record-caps-1')
$Tape = Join-Path $Workspace 'receipts/echo.tape.json' # 新文件，不能覆盖
& $Cli record --sdk $Sdk --worker $Worker --package $Package --workspace $Workspace --config $ConfigPath --input $RecordInput --output (Join-Path $Workspace 'receipts/record.json') --transcript $Tape --seed 17 --now-unix-ms 0
if ($LASTEXITCODE -ne 0) { throw 'record failed' }
& $Cli replay --sdk $Sdk --worker $Worker --package $Package --workspace $Workspace --config $ConfigPath --transcript $Tape --output (Join-Path $Workspace 'receipts/replay.json')
if ($LASTEXITCODE -ne 0) { throw 'replay failed' }
~~~

原始鉴权头、显式密钥和 credential.sign 交互不能录成有效 tape；工具拒绝记录，不通过打码后改变原交互来假装精确 replay。真实服务或真实厂商的调用不得因为生成了 tape 就标为 LIVE 验收。

conform 读取单独证据 manifest，不接受 --input 或 --transcript。每项声明字段为 id/status/mode/command/exitCode/artifacts/reason；artifact 是相对 evidence-root 的 path、真实 sha256 与 EVIDENCE/TRANSCRIPT kind。缺 required case、NOT_RUN、非零退出、缺/改证据都不能全量通过。下面故意生成全 NOT_RUN 声明，命令应非零，用于检查缺证据拒绝：

~~~powershell
$EvidenceRoot = Join-Path $Root 'evidence'
New-Item -ItemType Directory -Path $EvidenceRoot | Out-Null
$CaseCatalog = Get-Content -Raw (Join-Path $Sdk 'protocol/conformance-cases.json') | ConvertFrom-Json
$Cases = @($CaseCatalog.cases | ForEach-Object {
  @{id=$_.id;status='NOT_RUN';mode='MOCK';command=@();exitCode=$null;artifacts=@();reason='尚未执行，不声明通过'}
})
$EvidenceManifest = Join-Path $EvidenceRoot 'manifest.json'
Save-Json $EvidenceManifest @{schemaVersion=1;cases=$Cases}
& $Cli conform --sdk $Sdk --worker $Worker --package $Package --workspace $Workspace --config $ConfigPath --manifest $EvidenceManifest --evidence-root $EvidenceRoot --output (Join-Path $Workspace 'receipts/conform.json')
if ($LASTEXITCODE -eq 0) { throw '缺证据不应返回成功' }
~~~

command/exitCode 是证据声明，conform 不代替你执行所写命令；EVIDENCE_DECLARATION 与实际重放的 ACTUALLY_EXECUTED 必须区分。C28 必须有真正执行的匹配 tape。即使完整性/重放检查通过，也不能把声明字段当完整 36 case 的实际测试或独立作者证明。[完整验收条件](CONFORMANCE.zh-CN.md)还要求真实故障矩阵、隔离作者和候选复现材料。

## 7 发行与许可边界

本稿尚未提供已验收的 export_sdk.py、verify_release.py、check_author_evidence.py 发行工作链；不要把这些未来工具名当作当前可用 CLI 子命令。候选导出、G00–G06、C01–C36 及最终 SDK_READY 必须由完整真实证据决定；局部 --self-test 通过不能替代后续独立作者和重现。

本树使用 LicenseRef-Proprietary 标识，本文不授予 MIT、Apache、开源、分发、商业使用或再授权等新权利；适用权利须由权利人另行明确。依赖锁不是许可证清单，候选发行仍须审查并附依赖来源与许可证材料。SDK 就绪也不代表任何生产模块已经接入。
