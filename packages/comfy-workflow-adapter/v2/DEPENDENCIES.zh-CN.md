# 本样例的运行依赖

作者机器：Windows x64、PowerShell 7、Rust 1.95.0、wasm32-wasip2、MSVC原生工具链。公开CLI与worker随开发包提供；首次构建要显式联网下载锁定的公开crate，后续离线依赖其缓存。本包不安装Python、CUDA、模型或ComfyUI。

CLI真实素材检查需要明确传入可运行的 ffprobe，并由用户先将授权素材复制到显式 workspace 内；直接从工作区外导入会被宿主拒绝。驱动视频应为与 request 的 sourceRange 长度一致、从0开始且fps匹配的已准备素材。默认请求为3秒/24fps/640×640/6步，样例参数可显式编辑，不代表所有GPU都能承受所有合法尺寸。

ComfyUI端的 Move 操作需要以下固定文件（来自原包 mapping.rs）：

- UNet：`Wan2_2-Animate-14B_fp8_e4m3fn_scaled_KJ.safetensors`。
- LoRA：`lightx2v_I2V_14B_480p_cfg_step_distill_rank64_bf16.safetensors`、`WanAnimate_relight_lora_fp16.safetensors`。
- 文本编码器：`umt5_xxl_fp8_e4m3fn_scaled.safetensors`。
- VAE：`wan_2.1_vae.safetensors`；CLIP Vision：`clip_vision_h.safetensors`。
- DWPreprocessor：`yolox_l.onnx`、`dw-ll_ucoco_384_bs5.torchscript.pt`。

主要节点：UNETLoader、LoraLoaderModelOnly、ModelSamplingSD3、CLIPLoader、VAELoader、CLIPVisionLoader/Encode、CLIPTextEncode、LoadImage、LoadVideo/GetVideoComponents、ImageScale、PixelPerfectResolution、DWPreprocessor、WanAnimateToVideo、KSampler、TrimVideoLatent、VAEDecode、ImageFromBatch/ImageBatch、CreateVideo/SaveVideo。具体输入名与枚举必须对照目标服务的 `/object_info`。

Move 不使用 SAM2、人物点选、遮罩或保留原视频背景。包源码中保留的 Mix 操作额外依赖 SAM2、分割及遮罩节点，本阶段未将它列为实跑验收出口。

连通检查只访问 ComfyUI system_stats，不能证明模型和节点齐备。正式提交前应使用目标服务的节点定义确认所用类及固定文件名；缺项明确报告，不自动下载、替换模型或改用其他工作流。原生服务错误与SDK错误分别记录。
