# Qwen Image 2.1 单图编辑作者样例（P02）

本包使用冻结的公开 SDK v2，能力为 image.generate，操作为 local.example.comfy-qwen-i2i/qwen-i2i。只接收一张原图和编辑文字，返回一张 PNG；缺图、额外图片或失效媒体授权明确失败，不改走文生图，不复制原图补位。P01 文生图包独立保留。2026-10-23 开发版 P02 验收通过：独立构建/打包检查、软件安装关联、真实单图编辑、原字节上传、成果回收与原图另存、正常退出重开后的原 ID 续办均已实测。验收过程的停工、环境及门禁失败与获授权修正保留在项目分阶段计划 §9。P11已完成包含本包的完整NSIS资料交付；本段P02实跑仍按其原开发版基线记录，不承诺更早旧程序支持。

## 独立构建

开发者执行 Export-QwenImageEdit.ps1，显式传入已有 SDK CLI、worker 和不存在的绝对输出目录。导出的资料包含公开 SDK、源码、脚本、Canvas 原件及原生 API 图；构建不读取主程序源码。

```powershell
$Kit = 'E:/qwen-single-edit-kit'
& "$Kit/New-ComfyExample.ps1" -Project 'E:/qwen-single-edit-work/author'
& "$Kit/Build-ComfyExample.ps1" -Project 'E:/qwen-single-edit-work/author' -OutputDirectory 'E:/qwen-single-edit-work/build-1'
```

需要 Rust 1.95.0、wasm32-wasip2 和本机 MSVC 工具链。首次显式联网获取锁定依赖，缓存齐备后才用 -Offline。输出父目录先存在，输出目录本身不得存在。runtime/ 为标准 SDK 包，install/ 额外包含原件关联清单。

## 输入与真实图

根字段沿用 P01 的 prompt、width、height、seed、negativePrompt、advanced.steps，另有必填 referenceImages 数组，恰好一个 {mediaId} 对象。宽高为 256–4096、8 的倍数；steps 为 1–50；seed 为 0–9007199254740991。输入身份经操作声明的媒体槽和 SDK media grant 绑定，文件内容不缩放、不重编码，文件扩展名必须符合真实格式。此宿主输入支持 PNG、JPEG、WebP。

单图 Canvas 由仓内 image_qwen_image_2_1_image_edit.json 派生：删除第二个 LoadImage，断开第二个可选图片输入，保留第一图经 VAE 和 TextEncodeQwenImage21 的真实条件连线；关闭提示词重写，启用用户确认的 custom_size。API 图来自原生 ComfyUI 的“导出 (API)”操作，不调用私有转换器。SOURCE.json 记录原件、变体与导出摘要。原工作流未覆盖，后续多图操作另行验收。

适配包通过 SDK 的媒体租约及 multipart 上传原图，上传回执只绑定到 LoadImage 470；随后只提交一次 prompt，受理即 checkpoint，恢复只 query 原 ID。图片回收、尺寸核对和错误合同复用 P01。服务需具备原件声明的 Qwen Image 2.1 节点及模型，包括 qwen_image_2.1_int8_convrot、qwen3vl_8b_int8_convrot、Qwen VAE 和图中声明的 PE 模型；完整名称以图为准。

## CLI 与软件

```powershell
$Runtime = 'E:/qwen-single-edit-work/build-1/runtime'
$Workspace = 'E:/qwen-single-edit-work/live'
$Endpoint = 'http://YOUR-COMFY-HOST:8188'
& "$Kit/Invoke-QwenImageEdit.ps1" -Action caps -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint
& "$Kit/Invoke-QwenImageEdit.ps1" -Action probe -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint
& "$Kit/Invoke-QwenImageEdit.ps1" -Action submit -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -RequestFile "$Kit/request-i2i.json" -ReferenceFile 'E:/images/original.png'
# 只使用首次 submit 返回的原 requestId；不自动重提。
& "$Kit/Invoke-QwenImageEdit.ps1" -Action query -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -SubmitRequestId 'ORIGINAL-SUBMIT-REQUEST-ID'
```

CLI 示例只配置匿名局域网，不覆盖需要鉴权的部署。安装 install/ 目录，导入 qwen-image-2.1-single-edit.source.json 并关联本操作。若此前服务离线导致原生派生未完成，连接后使用工作流库已有的“检查并更新”；确认已加入图生图能力列表。在自由画布连接一张原图和编辑文字，选择该工作流，确认参数后提交。软件须包含 P02 的图片媒体槽消费；仅支持 P01 的程序会明确拒绝本操作。当前入口为自由画布；CLI、真实软件及安装版验收分别记录，完整发行资料在 P11 验证。

不支持远端取消，不调用全局 interrupt。失败保留原图和既有历史，不静默换通道。单次多输出、软件批量、视频、音频及多图模式不在本步验收范围。
