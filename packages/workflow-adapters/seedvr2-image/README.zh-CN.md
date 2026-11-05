# SeedVR2 单张图片放大适配包

基于固定的 SDK 2.0.0 用户预览版，仅用于一张原图到一张放大图片。声明已有 `imageUpscale` 传输槽；能力使用 SDK 正式支持的命名空间扩展 `local.bgx.seedvr2-image/upscale`，不声称尚未发布的 `image.upscale` 标准键。软件消费按传输槽和操作声明判断，不按包名分派。

源工作流为仓内认证文件 `ComfyUI-API/SeedVR2/image_seedvr2_simple_upscale.json` 的原始字节副本 `workflow/source.json`；`workflow-templates.json` 固定其摘要。`src/workflow-api.json` 是作者对该图的显式 API 投影，保留原图中 alpha/torch-compile 的 Bypass：一张 LoadImage，连接的 DiT/VAE，单帧 SeedVR2，唯一 SaveImage。模型、节点、字段、HTTP和输出选择均在此包内，不进入SDK。

支持当前图片工作台的短边/最长边、种子、增强模型、颜色、噪声、VAE分块、换出与卸载参数。PNG没有视频CRF，此操作对 `writerCrf` 明确拒绝，不能静默忽略。任务须由标准SDK上传原图、提交、保存受理checkpoint，再按原远端任务号查询；不自动重提，不调用ComfyUI全局interrupt。

## 构建与安装

使用 PowerShell 7 和 SDK 要求的 Rust 1.95.0 / wasm32-wasip2。运行 `Build.ps1`，显式提供 `-Sdk` 公开源码根、`-RuntimeSdk` 固定运行布局根、`-Cli`、`-Worker` 与新的 `-OutputDirectory`；首次作者缓存需联网，已有完整锁定缓存才可加 `-Offline`。输出 `runtime/` 是SDK严格运行包；`install/` 额外携带工作流原件关联清单，供软件安装。

在「设置→第三方适配器」安装 `install/`，按现有完整host-config合同配置ComfyUI连接、网络授权和FFprobe；不要将只有endpoint的对象当完整配置。随后导入 `workflow/source.json`，在工作流详情关联本包的“SeedVR2 单张图片放大”，并加入“图片高清放大”能力。软件选择的服务须与包的唯一网络授权地址一致；不一致直接报错。

需要包含本专项图片放大消费者的新开发程序；0.6.1030及本专项之前的程序没有这条消费路径，不能用安装包卡片可见代替实际支持。从图片节点的高清放大工作台选择该工作流，确认参数后提交；成果复用现有单张放大的回写、历史、原图保存。当前专项不接视频或批量；新包不自动切换既有未关联工作流的执行路径。

## 验收边界

标准构建、包检查和打包已执行；软件链路与最终测试状态以 `docs/dev/reports/seedvr2-image-v2/20261025-SCOPE.md` 的实际记录为准。CLI/软件/安装发行分开记录，不因此修改SDK_READY或宣称全SDK正式封存。
