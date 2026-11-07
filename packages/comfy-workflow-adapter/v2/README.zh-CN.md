# ComfyUI 作者开发包：图片、视频生成与视频编辑

开发者预览版随安装发行0.6.1030交付；状态与使用边界见 [预览版说明](../../../contract/v2/DEVELOPER-PREVIEW.zh-CN.md)。SDK合同2.0.0和执行工具未变；主程序修复与本版实际验收以0.6.1030发行回执为准。当前完整输入模式与P01–P13最终边界请先读[生成作者索引](GENERATION-EXAMPLES.zh-CN.md)，包含P12 FLUX2与P13 H3 Continuum独立作者新图。下文保留Wan编辑样例操作指南。

图片生成作者请先看 [Qwen Image 2.1 文生图样例](qwen-t2i/README.zh-CN.md)。它使用同一公开 SDK；图片工作台消费范围与下方视频编辑表单分别说明。

其他生成样例：[单图编辑](qwen-i2i/README.zh-CN.md)、[双参考图](qwen-multi-ref/README.zh-CN.md)、[单任务双图输出](qwen-two-output/README.zh-CN.md)、[H3文生视频](h3-t2v/README.zh-CN.md)、[H3双参考图生视频](h3-r2v/README.zh-CN.md)。这些与Wan视频编辑共用冻结SDK，模型节点和HTTP留在各自包中。视频生成目前从自由画布工作台消费，支持边界以各包声明为准。[LTX2.5单图生视频](ltx25-i2v/README.zh-CN.md)P07开发版真实链路已通过：独立编译、安装关联、原图上传、出片回收、原ID退出续办及播放另存；P11已完成实际安装目录取件验收；当前收尾发行补入P12/P13新样例，准确版本及资料核验以随包说明为准。

本包面向 Windows x64 / PowerShell 7，演示用公开 SDK v2 制作 Rust/WASM 适配包。样例源码从现有 Wan 2.2 包生成，身份为 `local.example.comfy-wan22@1.0.0`，避免与内置包重名。第一阶段主要验收 `/move`：角色图片按驱动视频动作表演；源码仍保留原包的 Mix 操作，但不据此宣称 Mix 已完成本阶段实测。

**离线编译、包检查、CLI 出片、软件内安装关联与实际运行须分别验收。重开回收和实际安装版交付也须各有实测记录；仅构建或包检查通过不能代替软件可用。**

普通工作流若已经可由工作流库导入、映射、发布和绑定使用，直接复用该路径；需要独立构图/操作逻辑时才参考本样例。这里没有新的 Canvas 转换器或节点自动猜测。样例里的 [sample-source/workflow/source.json](qwen-t2i/sample-source/workflow/source.json) 是保留原始字节的认证来源画布。本目录没有统一的 src/graph.rs；API 图在各样例自己的源码里构造。

## 1. 目录和环境

独立交付目录包含 `vendor-adapter-v2/`（公开 SDK 源码、锁、schema、模板及 bin 内 CLI/worker）、`sample-source/`、三个操作脚本、请求样例和作者任务书；不需要主仓、主程序源码或用户数据目录。`SOURCE.json` 记录生成来源与实际摘要，当前工程用于开发验收，不更改 SDK 全局发行状态或许可。

需要安装 Rust 1.95.0、wasm32-wasip2 target 和本机 MSVC 原生编译工具。首次作者构建显式联网获取锁定依赖；缓存齐备后才可使用 `-Offline`。SDK 会拒绝祖先 `.cargo/config` 或作者专用 Cargo home 中的隐式配置，不应删除用户配置绕过，应选择干净的仓外工作目录。

以下路径是用户自行选择的例子，替换为实际绝对路径。首次生成时作者工程和输出目录必须不存在；不覆盖之前成果。

```powershell
$Kit = 'D:/comfy-author-kit'
$Project = 'D:/comfy-author-work/author'
$Output = 'D:/comfy-author-work/build-1'
& "$Kit/New-ComfyExample.ps1" -Project $Project
& "$Kit/Build-ComfyExample.ps1" -Project $Project -OutputDirectory $Output
# 之后需要新构建目录时才执行；同一有效结果无需重复构建。
# & "$Kit/Build-ComfyExample.ps1" -Project $Project -OutputDirectory 'D:/comfy-author-work/build-2' -Offline
```

`build-1/runtime/` 是标准 SDK runtime 包；`build-1/install/` 是应用安装交付目录，额外包含 `workflow-templates.json`。标准 runtime 签名不覆盖旁置模板清单。不要将 install 目录作为严格 runtime 根重新检查。

## 现有视频编辑宿主的输入范围

SDK 的通用输入 schema 不等于应用视频编辑表单能消费任意字段。当前宿主提交的标准根字段为 `source`、`sourceRange`、按声明绑定的媒体槽、可选 `targetPoints`、声明过的 `prompt` / `negativePrompt`、`width`、`height`、`fps`、`audio` 和 `advanced`。自定义可调标量参数必须放在 `advanced.properties` 内，并在该闭集内声明类型、必填及约束；例如第二段动作描述用 `advanced.motionPrompt`，不能另建根级 `motionPrompt` 并期待宿主自动显示它。

现有参数表单从 `advanced` 的属性声明生成控件，字段可用 schema `title` 命名。额外根级必填字段即使通过通用 SDK 检查、CLI 可显式填写，也不能据此判应用支持。作者应在打包前核对必填字段都能由当前宿主表达；发现超出支持范围时先报告。不得把缺字段改成可选、默认填充或放宽校验来通过验收。软件安装/关联通过仍须另验实际表单提交和成果回收。

## 2. 离线检查参数确实进入图

生成工程保留原行为测试，覆盖参数拒绝、分段、连线、Move/Mix差异。[inspect.rs](resources/inspect.rs) 使用同一 parser/planner/builder 输出实际 Move API 图，不复制业务规则。它在本仓库的 `resources/`，不是仓库根上的 `examples/`。

```powershell
# 在作者目录执行，显式使用已经由 SDK build 获取的缓存；仅影响当前命令进程环境。
$savedCargoHome = $env:CARGO_HOME
$savedCargoTarget = $env:CARGO_TARGET_DIR
$env:CARGO_HOME = "$Project/.bgx-sdk/cargo-home"
$env:CARGO_TARGET_DIR = "$Project/.bgx-sdk/native-target"
Push-Location $Project
try {
  cargo +1.95.0 test --lib --locked --offline
  cargo +1.95.0 run --example inspect --locked --offline -- "$Kit/request-move.json"
} finally {
  Pop-Location
  $env:CARGO_HOME = $savedCargoHome
  $env:CARGO_TARGET_DIR = $savedCargoTarget
}
```

检查输出图的宽高、采样步数、seed、最终 fps 和目标帧数与请求一致。此步骤没有连接 ComfyUI，不能证明模型/节点齐备或生成成功。脚本不会自动补缺失参数或修正非法数值。

## 3. 局域网真实运行

先读 [依赖与素材要求](DEPENDENCIES.zh-CN.md)，让服务提供者确认节点和模型齐备。此脚本覆盖明确提供的无鉴权 HTTP(S) 服务；需要鉴权时应按公开 SDK 凭据 provisioning 合同显式配置，不将密钥写入源码/工作流。

```powershell
$Endpoint = 'http://YOUR-LAN-HOST:8188'
$Workspace = 'D:/comfy-author-work/live-1'
$Runtime = "$Output/runtime"
$Ffprobe = 'C:/tools/ffprobe.exe' # 实际可用路径
& "$Kit/Invoke-ComfyExample.ps1" -Action caps -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -ProbeExecutable $Ffprobe
& "$Kit/Invoke-ComfyExample.ps1" -Action probe -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -ProbeExecutable $Ffprobe
# 驱动视频必须已经裁剪为请求范围并转换为请求fps；CLI样例不冒充主程序素材预处理。
# SDK只允许从显式工作区导入素材。先由用户将授权文件复制进来，不放宽宿主权限。
New-Item -ItemType Directory -Path "$Workspace/media" | Out-Null
Copy-Item 'D:/media/source-3s-24fps.mp4' "$Workspace/media/source.mp4"
Copy-Item 'D:/media/character.jpg' "$Workspace/media/character.jpg"
& "$Kit/Invoke-ComfyExample.ps1" -Action submit -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -ProbeExecutable $Ffprobe -RequestFile "$Kit/request-move.json" -SourceVideo "$Workspace/media/source.mp4" -CharacterImage "$Workspace/media/character.jpg"
# 将上一步实际返回的 requestId 填入；不要再次submit。
& "$Kit/Invoke-ComfyExample.ps1" -Action query -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -ProbeExecutable $Ffprobe -SubmitRequestId 'comfy-submit-ACTUAL-ID'
```

query 显示 queued/running 时，按同一 SubmitRequestId 再 query；succeeded 后停止查询，按 outputs 中实际返回的 `media.fileName` 在工作区 `objects/` 定位视频，并用 ffprobe 检查真实时长/帧率/尺寸。CLI工作区保存原始输入、回执、诊断和内容寻址对象；关闭命令窗口不取消远端任务。失败或受理未知时不自动重提；本样例不支持远端取消，也不调用全局 interrupt。

本样例的 resumeToken 是原生 promptId，查询不会轮换该token；其他包必须遵守其自己的 token 更新合同。完整应用安装配置输出在 workspace 的 `host-config.app.json`，包含网络授权、传输、预算与 ffprobe；不是只填 endpoint 的两字段配置。在设置「第三方适配器」展开「高级安装选项」填写该完整配置，点击「安装适配包」安装 `build/install` 目录，再在工作流详情关联准确原件和操作。CLI 成功后仍须验证实际软件提交、成果回收、播放和保存。

## 4. 修改自己的工作流

交给编程智能体 [作者任务书](AGENT-TASK.zh-CN.md)、用户工作流、节点定义及明确输入/输出需求。优先改作者工程的 schema、capability、parameters、graph 和包身份；通用 SDK、密封 vendor-sdk 镜像及主程序保持固定。新图或新逻辑使用新包身份/版本、重新计算原件摘要并显式关联，旧任务不换包继续执行。

本样例固定 Wan 模型与图语义，不是任意工作流解释器。扩展媒体/交互原语前先确认当前产品是否支持；不能靠字符串容纳所有参数或新增主程序厂商分支来声称独立接入。

维护者在仓内通过 `Export-ComfyExample.ps1 -OutputDirectory <新绝对目录> -Cli <固定CLI> -Worker <固定worker>` 重新生成。不要手改 sample-source 形成第二份生产实现。导出不触发 SDK 全量发行证明或安装器构建。


多参样例：[H3文字＋双图＋原音](h3-r2v-audio/README.zh-CN.md)已完成P09A独立作者包/CLI及P09B开发版软件验收。包含P09B消费者的新程序可安装关联、提交两张有序原图和一份完整8秒WAV，经真实生成、关闭面板回收、正常退出同号续办及播放/原片另存验证。用途须显式确认soundtrack_copy，原音既作生成参考又复用为成片音轨；不代表口型或联合生成音轨已验。旧P08程序仍不支持音频槽。SDK冻结，P11安装资料及代表软件链路已验，详细限制及实际程序版本见样例说明。

另一模型多参样例：[LTX2.3文字＋单图＋参考音](ltx23-ia2v/README.zh-CN.md)已完成P10独立构建/包检查及开发版软件安装关联、真实生成、回收、正常退出同号续办和播放/原片另存。固定P09B宿主和SDK，无LTX宿主特判；现有参考音用途voice_timbre保留原生音频编码条件及解码音轨，不能宣称原音直通。实际193帧视频8.041667秒、音轨7.970秒，精确限制见样例说明；P11实际安装取件与LTX2.3代表链路已通过，后续原始记录保留。
