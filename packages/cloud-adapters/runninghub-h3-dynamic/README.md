# RunningHub H3 Turbo 动态多参独立适配器



> Spectrum 质量优先孪生包：`../runninghub-h3-spectrum-dynamic`（modelId `runninghub-h3-r2v-spectrum-dynamic`，workflowId `2116576147646533634`）。本 Turbo 包 workflowId `2100758868111486978` 保持冻结。
当前权威源码：**1.2.6**；公开 SDK：**V2 2.0.0**。本目录与 `packages/cloud-adapters/` 内其他厂商包同级，具有自己的 Cargo workspace，不加入宿主或 SDK workspace，不参与主程序编译。厂商工作流、HTTP 组装及响应解释只在本目录实现。

## 其他智能体从哪里开始

- `src/lib.rs`：能力声明、平台参数映射、受管上传、提交和原任务续查。
- `src/workflow-api.json`：包内完整工作流；不是宿主内置图，也不是待用户导入的 ComfyUI 工作流。
- `schemas/`：配置、输入、输出和续查合同。
- `manifest.json`、`Cargo.toml`、`Cargo.lock`：当前包身份和锁定依赖。
- `vendor-sdk/`、`template-lock.json`：冻结的公开 SDK 镜像；禁止手改。公开说明见[SDK V2 文档](../../../contract/v2/README.md)。这是开发者预览，不是已封存的稳定 SDK。
- `installation/adapter-install.template.json`：宿主真实读取的完整安装声明模板；只含公开配置及账号代理声明。
- `SOURCE.json`：当前源码身份、文件摘要和 1.2.2 基础来源。只保留最新源码，历史从 Git 追溯。

当前声明：1～9 张有序图片、0～3 段参考音；时长 1～30 秒；8 种画幅、0.1～16 MP、原生 24 fps。具体字段及边界以运行时 caps 和 schema 为准。参考音是生成条件，输出是模型生成音轨，不冒称参考原音直通。

## 独立构建与打包

准备公开的 SDK 2.0.0 作者分发目录（包含 `bin/bgx-adapter-v2.exe` 与 `bin/bgx-adapter-v2-worker.exe`），然后在 PowerShell 执行：

```powershell
& ./Build.ps1 -Sdk '<公开SDK的绝对路径>' -OutputDirectory '<尚不存在的输出目录>'
```

默认使用离线依赖缓存；首次没有缓存时，明确加 `-Online`。脚本依次执行公开 SDK 的 build、源码 check、pack 和运行包 check，不调用宿主编译，也不自动安装。输出为该输出目录中的 adapter-install.json 和 package 子目录；安装声明引用打包后真实 manifest 的 SHA256。Sdk 和 OutputDirectory 都使用绝对路径；OutputDirectory 必须尚不存在。生成的完整输出目录才是可安装包；选择其中 `adapter-install.json` 安装。不要只复制 WASM 或把源码目录直接当安装包。

原生行为回归可以独立运行：

```powershell
cargo test --manifest-path ./Cargo.toml --locked --offline --target-dir ./.bgx-sdk/native-tests
```

缺少依赖缓存或工具链时应先按 SDK 作者说明准备环境，不能静默改锁文件或切换 SDK 版本。构建产物和缓存均已忽略。

## 配置和安全边界

运行配置只包含 `endpointBaseUrl`、`credentialRef`、`instanceType`。真实 Token 由应用的凭据代理提供；不写入源码、配置示例、工作流或包内。配置 schema 仅声明合法地址和凭据引用，不是一个包含密钥的配置文件。

宿主必须区分明确的安装包来源与 ComfyUI 工作流来源。2026-10-28 的秒失败 `COMFY_ENDPOINT_URL_MISSING` 已定责为宿主在 SDK submit 前错误执行 ComfyUI 地址门控；不能通过在本包补一个假 ComfyUI 地址绕过。该宿主修复已进入主项目提交 `c906b1b2f`。宿主侧根因记录不在本公开仓库，本次未能读取产品仓原文。宿主修复后用户真实提交已被云端接收并生成原片；后续下载授权与 usage 合同故障见本次修复。公开行为以本 README、`schemas/` 与 `src/workflow-api.json` 为准。

## 1.2.4 真实视频成果与验证范围

1.2.3 补下载 origin 与 usage 命名空间后，真实任务 `ec85d58f-a24f-4fdc-9ecb-c3247bd1c8dc` 的云端任务 `2115469862699757570` 已成功并被 SDK 完整下载/探测。接口 `outputType` 和 HTTP Content-Type 虽为 MP4，字节 `ftyp` 实为 QuickTime；SDK 合法返回 `kind=video`、`mime=video/quicktime`、内容摘要文件名 `.mov`。包内额外 `mime==video/mp4` 误拒了这份成果。

按用户最新要求，1.2.4 以冻结 SDK 实际探测的视频事实为权威，不再重复设 MP4/MOV 或其他容器白名单，也不按平台 outputType 猜格式。保留完整公开 DTO、非零大小/时长/尺寸、SHA及安全内容寻址文件名校验；HTTP、媒体和 checkpoint 的所有消费返回都经公开 SDK DTO 解码。保留真实 MIME/扩展，不转码、不静默改上游图或用户参数。下载失败仍保留原远端任务号，不重发生成。

宿主还存在 SDK collect 只收 MP4、画布把全部 ftyp 命名为 MP4、旧 FC resume 硬写 MP4 的连续问题，已在通用宿主提交 `20e2acf7b` 同批根治，因此需配套更新主程序 EXE；只更新独立包不足以完成应用回写。SDK本身、离线运行时和应用版本号保持不变。

实际通过：8项已有原生行为回归扩展；标准公开 SDK build/check/pack/runtimecheck；真实 SDK HttpBroker 匿名下载原已付费任务→MediaBroker finish→FFprobe→完整公共 stat 和 query artifact/usage validator。测试 fixtures 中的 QuickTime、MP4、EBML stat 均来自真实 SDK 序列化，包含全部必填 nullable 字段；QA MP4/Matroska只是原片的本地无损 remux 副本，不冒称新云端任务。全部受管端口审计及已知 SDK EBML 分类边界的宿主 FIX 不在本公开仓库，本次未能读取产品仓原文。上面这段是迁入时已写明的公开结论。

可安装输出仍是 `adapter-install.json` + `package/` 完整目录，用户自行安装。原1.2.3 checkpoint绑定原版本/digest，安装1.2.4不会自动接管；已有原片已保全为 `.mov`，无需重新付费生成。新 EXE 配套后的完整工作台提交、自动回写和逐格式 GUI 播放尚未验，不能用标准包检查冒充。

## 官方接口本地资料

宿主里的 RunningHub 官方接口笔记（上传格式、LoadAudio、WAV 限制）没有随本仓库发布，本次未能读取产品仓原文。公开行为以本 README、`schemas/` 与 `src/workflow-api.json` 为准；官方合同变化时再按第三方文档核对，不要把缺失的宿主笔记补写成新的接口说明。

本次修复：schemaJson与摘要生成前统一LF；新版本不得覆盖已安装旧版本。生成的SDK镜像由当前公开CLI init恢复，不手改镜像或锁；包检查通过仍须安装与工作台提交验收。

## 1.2.6 上传与失败提示

整组素材上传和生成提交共用9分钟（原3分钟），查询/取回同为9分钟。单次上传最多3分钟；上传失败通过公开 submission-rejection-v1 写明生成尚未提交，保留用户素材及提示词。生成提交后丢失回执仍须先核对第三方任务，禁止自动重提。
