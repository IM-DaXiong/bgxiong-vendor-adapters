# Wan 2.2 Animate 工作流资产说明（W01 离线部分）

认证原件：`ComfyUI-API/wan2.2/video_wan2_2_14B_animate.json`（画布 JSON，57 节点、108 连线，83,597 字节，SHA-256 `80b11e45d14419579126ff9861f523120d9519412a6dae73f34982c6dd78adf5`），与计划附件逐字节一致。以后先改此认证源，再同步 `../bgxiong-ComfyUI/API/wan2.2/`。

运行图不嵌入作者示例值：`src/graph.rs` 按请求快照逐节点构造 API 图，原件只作为作者资产与模板版本锚点（`src/mapping.rs` 记录摘要和画布节点到运行角色的对应）。仓内画布→API 编译器（`src-tauri/src/comfyui/workflow/*`）依赖实机 `/object_info` 决定 widget→输入名，不能离线运行，所以离线阶段没有产出编译后的 API 图；W05 前由实机编译一次并与 `graph.rs` 输出比对。

## 共享段（只构造一次）

| 画布节点 | class_type | 运行角色 / 取值来源 |
|---|---|---|
| 20 | UNETLoader | `Wan2_2-Animate-14B_fp8_e4m3fn_scaled_KJ.safetensors`, `default`（模板版本固定） |
| 18 | LoraLoaderModelOnly | `lightx2v_I2V_14B_480p_cfg_step_distill_rank64_bf16.safetensors`，强度=`advanced.distillLoraStrength` |
| 99 | LoraLoaderModelOnly | `WanAnimate_relight_lora_fp16.safetensors`，强度=`advanced.relightLoraStrength` |
| 60 | ModelSamplingSD3 | shift=`advanced.samplingShift`（示例 8） |
| 2 | CLIPLoader | `umt5_xxl_fp8_e4m3fn_scaled.safetensors`, `wan`, `default` |
| 3 | VAELoader | `wan_2.1_vae.safetensors` |
| 4 / 9 | CLIPVisionLoader / CLIPVisionEncode | `clip_vision_h.safetensors`；crop=`none`；image←替换图 |
| 10 | LoadImage | 替换图片，值=上传返回的服务文件名 |
| 21 / 1 | CLIPTextEncode | `prompt` / `negativePrompt` |
| 145 / 23 | LoadVideo / GetVideoComponents | 已准备好的源视频（上传返回名）；输出 images(0)、audio(1) |
| 212 | ImageScale | lanczos，`width`×`height`，crop=center |
| 158 | PixelPerfectResolution | original_image←23.images，image_gen_width/height=`width`/`height`，`Just Resize` |
| 100 | DWPreprocessor（脸） | detect_hand/body=disable，face=enable，yolox_l.onnx，dw-ll_ucoco_384_bs5.torchscript.pt |
| 101 | DWPreprocessor（姿态） | hand/body=enable，face=disable |
| 108 | DownloadAndLoadSAM2Model | `sam2_hiera_base_plus.safetensors`, video, cuda, fp16 |
| 229 | PointsEditor | **运行图删除**：交互节点；其 positive_coords（STRING）由包直接写入 107.coordinates_positive |
| 107 | Sam2Segmentation | keep_model_loaded=true，individual_objects=false，image←212，coordinates_positive=包换算后的点 JSON |
| 274 / 276 | GrowMask / BlockifyMask | grow=`advanced.maskGrowPixels`、tapered=true；block=32、cpu |
| 275 | DrawMaskOnImage | `0, 0, 0`、gpu → background_video |
| 124/127/44/103/126/161/226/227/238 | 预览、说明 | 运行图删除 |

## 生成段（每段一组，K 由 planner 决定）

`WanAnimateToVideo` 输出槽：positive0 / negative1 / latent2 / trim_latent3 / trim_image4 / video_frame_offset5；widgets 顺序 width,height,length,batch_size,continue_motion_max_frames,video_frame_offset。
- length=C（`windowFrames`），continue_motion_max_frames=O（`overlapFrames`），batch_size=1；width/height=请求值。
- 第 1 段：不接 continue_motion，video_frame_offset=0。第 k>1 段：continue_motion←上一段累积 ImageBatch（首段为其 VAEDecode），video_frame_offset←上一段 WanAnimateToVideo 输出 5。
- KSampler（seed/steps/cfg/sampler/scheduler/denoise=1）→ TrimVideoLatent(trim_amount←输出3) → VAEDecode → ImageFromBatch(batch_index←输出4, length=4096)；k>1 用 ImageBatch 拼到累积帧。
- 原件三段末端各有 CreateVideo+SaveVideo（24/24/16 fps 不一致，第三组 mode=4 已停用）：运行图全部删除，只保留一个最终 ImageFromBatch(0, N) → CreateVideo(fps=`fps`, audio 视 `audio` 接 23.audio) → SaveVideo。

## 待实机 `/object_info` 确认（W05 前，用户执行）

1. 节点包与版本：KJNodes（PointsEditor/BlockifyMask/DrawMaskOnImage/ImageFromBatch 来源）、comfyui_controlnet_aux（DWPreprocessor、PixelPerfectResolution）、ComfyUI-segment-anything-2（Sam2Segmentation、DownloadAndLoadSAM2Model），以及核心 WanAnimateToVideo/TrimVideoLatent/CreateVideo/SaveVideo/LoadVideo 存在。
2. 各节点 widget→输入名与上表一致（尤其 DWPreprocessor 的 detect_hand/detect_body/detect_face/resolution/bbox_detector/pose_estimator/scale_stick_for_xinsr_cn、Sam2Segmentation 的 keep_model_loaded/individual_objects、GrowMask 的 expand/tapered_corners、BlockifyMask 的 block_size/device、DrawMaskOnImage 的 color/device、CLIPVisionEncode 的 crop）。
3. Sam2Segmentation.coordinates_positive 接受 `[{"x":..,"y":..}]` 字符串直输（不经 PointsEditor）。
4. WanAnimateToVideo：length 是否要求 4n+1、continue_motion_max_frames 是否要求 4n+1、条件序列短于 G 时尾部如何补齐；trim_latent/trim_image/video_frame_offset 输出语义与 planner 公式一致（k 段起点 = (k-1)(C-O)）。
5. width/height 16 倍数、CreateVideo fps 支持范围；sampler/scheduler 枚举与 schema 一致。
6. 模型文件全部在位：上表 5 个模型 + yolox_l.onnx、dw-ll_ucoco_384_bs5.torchscript.pt、sam2_hiera_base_plus.safetensors。
7. `/upload/image` 可上传 mp4 并被 LoadVideo 读取；SaveVideo 在 `/history` 中的输出键（images + animated）。
## 驱动表演（Move）操作 `bgx.workflow.wan22-animate/move`

- 输入 schema `input-move`：没有 `targetPoints`，也没有 `advanced.maskGrowPixels`，传了就拒绝。`source` / `replacement` 和替换（Mix）操作相同。
- 构图差异：每段 `WanAnimateToVideo` 都不连 `background_video` / `character_mask`。SAM2 加载与分割（108/107）、PointsEditor、GrowMask/BlockifyMask/DrawMaskOnImage（274/276/275）这几个节点**不生成**，也不读源视频宽高。姿态/脸（100/101）、CLIP Vision、续接、末端 CreateVideo/SaveVideo 和 Mix 一致。
- 语义：输出是“替换图中的人物按源视频动作表演”，背景来自替换图，不保留源视频画面。
- 模板版本、模型文件、默认值（1280×720、24fps、C=77、O=5、steps 6、cfg 1、seed 0）与 Mix 相同，同一个 `workflow-templates.json` 锚点。

### Move 待实机确认（W05，用户执行）

1. 不连 background_video/character_mask 时 WanAnimateToVideo 能正常排队（不报缺输入），出片人物跟随源动作。
2. 记录 Mix、Move 各一次 1280×720/24fps/C=77 的 4090 显存峰值，回填两个 schema 的 `x-resourceHint.vramPeakBytes`。
3. 关掉 SAM2 相关节点后，`/object_info` 中缺少 segment-anything-2 节点包也能跑 Move（Mix 仍需要）。

## 任意帧选人与双向遮罩（0.1.2）

### 0.1.3 修正 SAM2 遮罩通道

SAM2 传播返回 `[objects, 1, H, W]`；包属节点逐对象取第零通道再合并，输出严格为 `[N, H, W]`。0.1.2 在 NumPy 合并时保留通道维，导致双向跟踪完成后报 `mask at frame 0 must be 2-D`。13 项 companion 回归覆盖真实 torch logits 转换、单像素空间轴、多人并集、空遮罩和双向节点输出；完整人物替换效果仍须人工验收。

安装新版包及其同批 `comfy-nodes/BGXAnimateTracking` 后重启 ComfyUI，并在工作台选择 0.1.3；不要覆盖已安装的 0.1.2 文件或沿旧任务重试冒充新版。主程序与数据库没有因本修复变化。

- 人物替换（Mix）在 ImageScale 之后插入包属 BGXAnimateFrameSequence：输入裁切后 images、GetVideoComponents 的真实源 fps（FLOAT，端口 2）、目标 fps 与 N；按 floor(j * source_fps / target_fps) 采样，越界直接失败，不复制末帧。pose / face / SAM / background 均消费该唯一序列。
- Sam2Segmentation 替换为包属 BGXAnimateBidirectionalMask：在 a = floor(frameTimeMs * target_fps / 1000) 加点，正向传播到 N-1，再反向到 0，按 out_frame_idx 合并，锚点去重，严格输出 0..N-1。空遮罩可保留；缺帧/坏形状失败，不回退旧节点。
- Move 操作不加 sequence / SAM2 分支，仍使用 src.scaled。
- 节点 SSOT：companion/custom_nodes/BGXAnimateTracking/。部署：companion/install.ps1 -TargetComfyDir <绝对 Comfy 根目录>，然后重启目标 Comfy 并用 /object_info 核对。上传前适配器会做最小节点合同检查；缺 companion 明确失败。
- GetVideoComponents.fps 已核实为 FLOAT（Comfy core canvas / object_info）。缺少真实源 fps 时不得用输出 fps 冒充。
# 0.1.4 参考图准备

认证工作流原件及动态生成节点接线保持不变。独立适配器先调用自身companion准备接口，将可解码参考图校正方向、完整等比缩放并补RGB(127,127,127)背景至冻结生成画幅，再把派生PNG作为原有LoadImage输入。两条参考通道使用同一派生图；不是SDK/宿主统一策略，不修改源图、源视频或遮罩坐标。打包唯一入口为本目录 `build-package.py`，安装声明必须含reference.py等四个companion文件的真实摘要。详细边界见[使用说明](README.zh-CN.md)。


## 0.2.0 多关键帧跟踪与 maskArtifact

| 节点 | 角色 |
|---|---|
| BGXAnimateMultiKeyframeMask | mask-only 多关键帧 |
| BGXAnimateMaskArtifactExport | 原子导出 maskArtifact |
| BGXAnimateMaskArtifactLoad | Mix 读确认遮罩 |
| /bgx/wan22-animate/tracking/{id}/progress|manifest | WASM 进度/回执 |

## 资源预检（本轮）
- connectivity probe 会汇总伴随节点与生成模型可读性；缺生成模型不阻断仅需 SAM2 的跟踪。
- 伴随节点经 `/bgx/wan22-animate/companion/identity` 回报已加载身份，与安装包版本比对。

