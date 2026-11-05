# Wan Animate 2（蒸馏版）角色动画适配包 — 资产说明

认证原件：`ComfyUI-API/Wan-Animate-2/video_wan_animate2_distilled.json`（画布 JSON，SHA-256 `e08670cf0a14f4648e97283e4d7c78fe945015d5b11e4550e94b636a1f79324f`，与 `workflow-templates.json` 一致）。工作流库里导入的必须是这份原件（逐字节一致）才能关联本包。

## 用法

1. 设置 → 第三方适配器 → 「SDK v2 适配包」：宿主配置填 `{"endpointBaseUrl":"http://127.0.0.1:8188","credentialRef":null}`（或你的 ComfyUI 地址），选本目录安装（目录内需已有构建产出的 `adapter.wasm` 和 `manifest.sha256`）。
2. 工作流库 → 打开已导入的 Wan-Animate-2 原件 → 「关联适配器」→ 选「角色动画（动作迁移）」。
3. 视频工作台 → 视频编辑：选该操作，提供「驱动视频（动作来源）」和「角色图片（外观来源）」，填角色外观描述，按需展开高级参数。

## 必需资源（ComfyUI 端）

| 类型 | 名称 |
|---|---|
| 扩散模型 | `wan_animate_2_distill_int8_convrot.safetensors` |
| 文本编码 | `umt5_xxl_fp8_e4m3fn_scaled.safetensors`（type `wan`） |
| VAE | `Wan2_1_VAE_bf16.safetensors` |
| CLIP Vision | `clip_vision_h.safetensors` |
| 核心节点 | WanAnimate2ToVideo、WanAnimate2Cache、ContextWindowsManual、ModelSamplingSD3、BasicScheduler、KSamplerSelect、SamplerCustom、TrimVideoLatent、ResizeImageMaskNode、CLIPVisionEncode、GetVideoComponents、CreateVideo、SaveVideo、LoadVideo、LoadImage、ImageFromBatch、ImageBatch |

## 画布 → 运行图

- 子图「Motion Transfer」的共享部分只建一次：UNET →（可选）ContextWindowsManual(21/8/standard_static/pyramid) → WanAnimate2Cache(device/dtype) → ModelSamplingSD3(shift)；BasicScheduler 接 cache；驱动视频经 GetVideoComponents → ResizeImageMaskNode(scale dimensions, center, area)；角色图同尺寸缩放；两路 CLIPVisionEncode(crop none)。
- 角色外观提示词（画布 605 → 节点 3）进 `positive`；动作提示词（画布 604 → 节点 222）进 `positive_pose`，留空则不接、用节点默认。
- 每段 = WanAnimate2ToVideo → SamplerCustom → TrimVideoLatent(trim_latent) → VAEDecode。第 k>0 段：`continue_motion` ← 上一段解码帧，`video_frame_offset` ← 上一段输出 5，并去掉首帧（画布开关 `value_1=true` 的等价）后 ImageBatch 拼接。段数 K 由 `planner` 按目标帧数 N 与 `segmentFrames` 计算。
- 输出：ImageFromBatch(0, N) → CreateVideo(fps，`audio=source` 时接原视频音轨) → SaveVideo。画布里的左右对比（Video Stitch → 第二个 SaveVideo）不进运行图。

## 证据

- `cargo test`：参数严格解析、段规划、图结构（单段/多段链接/动作提示词缺省）与 schema 默认值合法性。
- `bgx-adapter-v2 check`：inventory / schema / signature-policy / wasm-capabilities / wasm-package-validation 全部 VALIDATED。
- 运行图（3 段 + 上下文窗口）对照实机 `/object_info`：类、必填输入、连线类型、枚举全部存在（仅文件名占位与通配类型差异）。实机执行留给 S6。