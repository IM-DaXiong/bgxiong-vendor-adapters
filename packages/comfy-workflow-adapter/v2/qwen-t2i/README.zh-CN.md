# Qwen Image 2.1 文生图作者样例（P01）

这是一个 `image.generate` 作者包，使用冻结的公开 SDK v2。当前目录的 sample-source 是唯一源码；导出资料后可在仓外独立编译，不需要主程序源码。支持范围为文字生成一张 PNG，不接受参考图片、视频或音频。多图输入、单任务多输出与软件批量属于后续步骤。

## 独立构建

开发者先执行 Export-QwenExample.ps1，显式指定当前 SDK CLI、worker 和一个不存在的绝对输出目录。用户从导出的资料目录执行：

```powershell
$Kit = 'E:/comfy-qwen-kit'
& "$Kit/New-ComfyExample.ps1" -Project 'E:/comfy-qwen-work/author'
& "$Kit/Build-ComfyExample.ps1" -Project 'E:/comfy-qwen-work/author' -OutputDirectory 'E:/comfy-qwen-work/build-1'
```

需要 Rust 1.95.0、wasm32-wasip2 target 和本机原生工具链。首次构建按锁定依赖显式联网；缓存齐备后才使用 `-Offline`。输出目录父目录须已存在，输出目录本身不得存在。`runtime/` 是标准 SDK 运行包，`install/` 额外带关联原件的 workflow-templates.json。

导出资料内的 Invoke-QwenExample.ps1 是公开 CLI 的薄调用脚本。先执行 caps、probe，再按需 submit 一次；后续 query 必须使用该次返回的原 SubmitRequestId，不自动重提。Endpoint 由用户提供，示例不含地址或凭据；此脚本的匿名局域网配置不覆盖需要鉴权的部署。

```powershell
$Runtime = 'E:/comfy-qwen-work/build-1/runtime'
$Workspace = 'E:/comfy-qwen-work/live'
$Endpoint = 'http://YOUR-COMFY-HOST:8188'
& "$Kit/Invoke-QwenExample.ps1" -Action caps -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint
& "$Kit/Invoke-QwenExample.ps1" -Action probe -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint
& "$Kit/Invoke-QwenExample.ps1" -Action submit -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -RequestFile "$Kit/request-t2i.json"
# 将首次 submit 回执中的 requestId 原样填入：
& "$Kit/Invoke-QwenExample.ps1" -Action query -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -SubmitRequestId 'ORIGINAL-SUBMIT-REQUEST-ID'
```

## 输入合同

根字段为 prompt（1–8000 字符）、width/height（256–4096 且为 8 的倍数）、seed（0–9007199254740991）、可选 negativePrompt（最多 8000 字符）、advanced。advanced 为闭集，必填 steps（整数 1–50）。示例见 request-t2i.json。默认值来自 schema，软件确认时冻结；作者不得在未消费的根字段里放新的必填参数。

运行只改提示词、latent 宽高、KSampler 的 seed/steps 和文本编码器负面词；模型、VAE、采样器、CFG、保存节点沿用已跑通的图。恢复令牌保存受理时的宽高，回收必须是真 PNG 且尺寸一致。所有 ComfyUI HTTP、节点和模型语义只存在于适配包。

## 依赖与原件

服务需安装 TextEncodeQwenImage21、QwenImage21Cache、ComfySwitchNode、SaveImageAdvanced 等原件声明的节点，及 qwen_image_2.1_int8_convrot.safetensors、qwen3vl_8b_int8_convrot.safetensors、qwen_image_2.1_vae_bf16.safetensors 等原件模型。完整依赖以 workflow/source.json 和 src/workflow-api.json 的实际字段为准；不能用节点类存在代替模型齐备。

SOURCE.json 记录已有真实运行 API 图的来源和摘要。此样例复用该 API 图，保留对应 Canvas 原件用于安装关联；作者构建不调用私有 Canvas 转换器。修改工作流须先取得其公开 API 导出，再调整包映射、测试和原件摘要，按版本重新打包。

## 软件验收边界

安装 install 目录，在 ComfyUI 工作流库导入 qwen-image-2.1-t2i.source.json，并显式关联该包的操作，加入相应连接的文生图列表。在自由画布图片工作台选择该工作流，确认尺寸、种子和采样步数后提交。图片通过现有节点历史、预览、保存收尾。

软件须包含 P01 的图片 v2 消费实现；旧 S4 主程序仅验过视频编辑，不能运行本样例。CLI 编译/检查、软件安装关联、真实生成、关闭面板回收、正常退出重开续办是分别验收的事项。当前资料不能单凭文件存在声明这些验收已通过。

包不支持远端取消，不调用全局 interrupt。受理后只 query 原任务；请求结果丢失不能自动重提。局域网地址通过安装时的连接配置提供，源码不含用户地址或凭据。

2026-10-22 P01 开发版实测：仓外独立编译、3 项作者测试、标准 SDK build/check/pack、软件安装原件关联、1024×1024 指定提示词出图及面板关闭回收已通过。1536×1536 新任务受理后正常退出，确认进程退出且远端仍运行；重开后以原远端 ID 回收，单任务只新增一个历史版本，另存原图 SHA256 与受管原图一致。匹配门禁及阶段同步状态见项目 PLAN §8。此次运行的是包含 P01 修改的新开发版，不是 NSIS 安装版；P11安装资料整体交付已通过；这段保留P01当时开发版来源。本包实测限单张文生图及自由画布；其它输入模式已有各自独立样例验收，见上级GENERATION-EXAMPLES索引。
