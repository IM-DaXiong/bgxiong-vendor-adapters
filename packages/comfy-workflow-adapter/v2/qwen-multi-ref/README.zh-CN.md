# Qwen Image 2.1 有序双参考图作者样例（P03）

使用冻结的公开 SDK v2，能力 image.generate，操作 local.example.comfy-qwen-multi-ref/qwen-multi-ref。恰好两张原图按会话顺序输入：第一张是编辑场景，第二张是参考图；返回一张真实 PNG。本包的两个 LoadImage 决定容量2，不是 SDK 或产品的统一上限。缺图、超量、重复媒体身份或租约不全显式失败，不改成单图或文生图，不拼图，不丢弃第二图。

## 独立构建

执行 Export-QwenImageEdit.ps1，显式传入已有 SDK CLI、worker 和不存在的绝对输出目录。资料含公开 SDK、源码、脚本、Canvas 原件和原生 API 图；构建不读取主程序源码。

```powershell
$Kit = 'E:/qwen-multi-reference-kit'
& "$Kit/New-ComfyExample.ps1" -Project 'E:/qwen-multi-reference-work/author'
& "$Kit/Build-ComfyExample.ps1" -Project 'E:/qwen-multi-reference-work/author' -OutputDirectory 'E:/qwen-multi-reference-work/build-1'
```

需 Rust 1.95.0、wasm32-wasip2 和 MSVC；锁定依赖先显式联网获取，缓存齐后才加 -Offline。输出父目录先存在，输出目录本身不得存在。runtime/ 是标准 SDK 包；install/ 含原件关联清单。

## 输入、顺序和真实图

prompt、negativePrompt、width、height、seed、advanced.steps 沿用单图例子。referenceImages 必填且恰好两个仅含 mediaId 的对象，按数组顺序对应 image_1、image_2。可在提示词中明确 image 1 / image 2 的角色。尺寸256–4096且为8的倍数，seed为0–9007199254740991，steps为1–50。PNG、JPEG、WebP按真实原字节上传，不使用缩略图或重编码。

workflow/source.json 由认证 image_qwen_image_2_1_image_edit.json 派生，只关闭提示词重写并启用明确的自定义尺寸，保留独立470/475图片入口；API 图从原生ComfyUI“导出(API)”取得，SOURCE.json记录摘要。两张租约完整校验后依次受管multipart上传，再只提交一次prompt。受理即checkpoint，后续仅query原ID。TextEncodeQwenImage21分别接收470到image_1、475到image_2，并使用原图像VAE条件。节点和模型名称以图为准，服务需有Qwen Image2.1配套节点/权重。不得用健康检查代替真实生成。

## CLI 和软件

```powershell
$Runtime = 'E:/qwen-multi-reference-work/build-1/runtime'
$Workspace = 'E:/qwen-multi-reference-work/live'
$Endpoint = 'http://YOUR-COMFY-HOST:8188'
& "$Kit/Invoke-QwenImageEdit.ps1" -Action caps -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint
& "$Kit/Invoke-QwenImageEdit.ps1" -Action probe -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint
& "$Kit/Invoke-QwenImageEdit.ps1" -Action submit -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -RequestFile "$Kit/request-i2i.json" -ReferenceFiles @('E:/images/scene.png','E:/images/reference.png')
& "$Kit/Invoke-QwenImageEdit.ps1" -Action query -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -SubmitRequestId 'ORIGINAL-SUBMIT-REQUEST-ID'
```

CLI例子仅覆盖匿名局域网。安装install/目录，导入qwen-image-2.1-multi-ref.source.json并关联本操作；服务在线时使用已有“检查并更新”，确认加入图生图。自由画布连接两张图到目标图片节点，在工作台参考图栏检查顺序；拖动重排是本次会话动作，“×”仅剔除本次参考，不删源节点或连线。补足两图、确认参数后提交；软件须包含P02有序图片媒体槽消费者。软件、CLI和NSIS发行验收各自记录，完整安装资料属于P11。P01和P02包独立保留。

不支持远端单任务取消，不调用全局interrupt。多输出、软件批量、视频和音频不属于本包。2026-10-23：仓外独立5项测试、标准build/check/pack、软件安装关联和双图真实采样已通过；在已有工作台重排为蓝书场景图→红书参考图后，分别上传原字节至image_1/image_2，取得1536×1536真PNG并自动回写，软件原图另存与受管原件一致。缺一张参考图的软件反例在出网前明确失败、无远端ID。P02已验的同一消费者回收/续办结论固定复用。阶段最终门禁和远程同步状态以项目分步计划§10为准，不据此冒称新NSIS安装版通过。
