# H3 Continuum 独立作者包

身份 `local.independent.comfy-h3-continuum@1.0.0`，操作 `local.independent.comfy-h3-continuum/continuum-driving-audio`，能力 `video.generate`。这是新的 Continuum 图，不是既有 H3 R2V 或 LTX 包改名；只复用公开样例的 SDK 端口、HTTP 和异步回收代码。

仅依赖已安装公开作者资料、用户提供的原件、ComfyUI 原生 API 导出和节点定义。没有读取宿主实现、仓库或其他阶段 QA；没有修改公开 SDK、CLI、worker、封存 vendor-sdk。实际阅读清单在 `logs/reading-list.txt`，冻结 SDK 全部文件摘要在 `logs/frozen-public-sdk.json`。

## 支持范围与音频语义

本步为固定最小档位：512×512、24fps、1×5 秒、Turbo 6 步。一张原图同时接入 `15.first_frame` 和 `15.reference_image_1`；一份从 0 秒开始的完整 5 秒 WAV 经 `4→15.driving_audio` 参与原生 H3 guide 条件，`15.driving_audio` 输出经 `18.driving_audio→19.audio` 原生装配保留。保留音频 VAE、逐段解码和装配支路；没有把 CreateVideo 改接 LoadAudio，没有裁剪、循环或换音。

必须显式选择 `soundtrack_copy`。目标节点定义明确说明该用途参与条件并保留原音；成片可能经过重采样/编码，不要求文件字节相同。本步授权音频是新合成非人声测试音，不证明口型、人声语义、跨段连续性或任意长音频。其他尺寸、帧率、段数和步数不在本包支持范围。原图可用 PNG/JPEG/WebP；音频仅 WAV，当前公开宿主每份最多 16MiB。

所有标量使用公开标准根字段或 `advanced`：`prompt` 原文写入 `5.value`；`seed` 写入 `15.base_seed`；宽高写入主控 `6/7.value`；fps 写入 `19.fps`；`advanced.chunks/chunkSeconds/steps/runName` 分别写入 `15.chunks/chunk_seconds`、`14.steps`、`15.run_name`。`durationSeconds` 必须等于段数×段长。`audioUsage` 仅作显式用途合同，不伪造图字段。

原生 `Save + Auto Resume` 保留。作者图固定新 `project_id=p13-independent-continuum-author-v1` 与 `filename_prefix=video/P13_Independent_H3_Continuum`；新生成应显式填写新的 `advanced.runName`，不要复用旧存储名称来冒充新采样。API 导出与画布原件的实际差异（如原生序列化的 diagnostics/strict_compatibility）按 API 导出保留，没有依据画布 widget 猜回去。

## 构建和安装

需要 PowerShell 7、Rust 1.95.0、wasm32-wasip2 和 MSVC 原生工具。SDK 及 CLI/worker 在 `public-kit/vendor-adapter-v2`。作者工程为 `author/`；仅在自己的 `.bgx-sdk` 新缓存按锁下载，不复制旧 target 或 WASM。

```powershell
$Root = 'E:/CodexWork/comfy-generation-p13-20261023/author-work'
& "$Root/Build-Continuum.ps1" -Project "$Root/author" -OutputDirectory "$Root/build-1"
```

脚本依次执行标准 build、check、pack、packed runtime check。输出必须尚不存在。有效结果不重复重建。`build-1/runtime` 是严格 SDK runtime；`build-1/install` 另外旁置 `workflow-templates.json`，不得把它当严格 runtime 根检查。旁置模板清单不在 runtime inventory 签名范围内。`author/workflow/source.json` 保留用户原件字节，安装关联认其真实 SHA-256。

设置→第三方适配器→SDK v2，填写完整 `host-config.app.json`，安装 `build-1/install`；在工作流库导入 `author/workflow/source.json` 并关联准确身份、版本和操作。在自由画布视频工作台选此工作流，一张原图和完整 5 秒 WAV，显式确认原音用途、runName 与固定档位后提交。`request-continuum.json` 是同一字段示例。参数默认显示不等于后端补值，缺失必填字段仍拒绝。

## 公开 CLI 与软件验收边界

`Invoke-Continuum.ps1` 提供 caps/probe/submit/query。独立作者仅运行无 GPU 的 caps/probe；软件实跑由父验收者进行一次。

```powershell
& "$Root/Invoke-Continuum.ps1" -Action caps -Runtime "$Root/build-1/runtime" -Workspace "$Root/cli-workspace" -Endpoint 'http://192.168.18.8:8188' -ProbeExecutable 'E:/CodexWork/comfy-generation-p12-20261023/app-data/runtimes/ffmpeg/20260607/ffprobe.exe'
& "$Root/Invoke-Continuum.ps1" -Action probe -Runtime "$Root/build-1/runtime" -Workspace "$Root/cli-workspace" -Endpoint 'http://192.168.18.8:8188' -ProbeExecutable 'E:/CodexWork/comfy-generation-p12-20261023/app-data/runtimes/ffmpeg/20260607/ffprobe.exe'
```

ffprobe 路径由验收者显式提供，只用于运行授权，不读取该阶段其他文件。更换机器须填实际路径和 endpoint。公开配置覆盖信任、网络、预算、传输与媒体探测，没有秘密或隐式凭据。无远端取消；受理后只沿原 submit requestId 查询，关闭页面不取消，不自动重提。软件安装、关联、真实提交、播放、原片保存、原号续办分别由实际回执证明，不能用包检查通过代替。

依赖：ComfyUI 支持 MiniMax H3 Native Masked；`H3ContinuumSamplerV34` / `H3ContinuumAssembleSeamV34`（ComfyUI-H3-Continuum）；`MiniMaxH3TurboLoRA` / `MiniMaxH3TurboSampler`；FL2VA 主模型 `minimax_h3_fl2va_pruned_int8_convrot.safetensors`、编码器 `qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors`、视频 VAE `minimax_h3_video_vae_fp16.safetensors`、音频 VAE `minimax_h3_audio_vae_fp32.safetensors`、Turbo LoRA `minimax_h3_turbo_v4_step600_ema.safetensors`。本包不安装或替换这些依赖。

## 实际状态

原生前提20节点通过：24514a98-64ac-4ba0-ace1-40ae92d090e8，63.494秒，0缓存节点、0复用段、1生成段；H.264512×512/24fps/120帧5秒、AAC48kHz双声道5秒。独立标准build/check/pack/packed-check一次通过；4项行为通过；CLI caps与connectivity probe ready。没有独立CLI GPU提交。准确摘要见 `AUTHOR-RESULT.json`，软件验收仍由父验收者单独报告。

首次写入准备脚本时 PowerShell here-string 嵌套导致命令解析失败，退出 1、没有写出脚本/改变源码；随后用文件补丁写入并执行成功。该次是作者脚本问题，没有称 SDK 或核心通过。原始命令在会话，实际执行脚本和后续日志保留。不因这次普通首次脚本错误扩大实现范围或重复整套验证。
