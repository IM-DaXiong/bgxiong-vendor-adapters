# A21 精修模型配套修复

## 原故障和归属
主线程真实任务 `7fd48943-69d5-4813-b9f6-738340eba935` / Comfy `c101c703-986d-44c9-9e50-f7a470a7026a` 已完成，但日志显示51组AdaLN LoRA补丁加载失败，因此该次不能记完整精修PASS。实际主模型 `minimax_h3_fl2va_pruned_int8_convrot.safetensors` 使用 `adaln_t_table[1025,8]`，block权重 `[96768,8]`；原配v4 LoRA的A `[16,2688]`、B `[96768,16]` 无法合并。Comfy `comfy/weight_adapter/lora.py` 的异常处理只记错误并返回原权重。不是SDK、宿主或普通显存不足。

## 精确来源与修复
锁定的FaceRefine 1.1.2示例使用较早名称 `minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors`。A21明确选择同一推荐作者Kijai发布的8-step rank-24 resized版本 `minimax_h3_fl2v_lightx2v_turbo_8step_v1.0_resized_avg_rank_24_bf16.safetensors`，不宣称与原示例字节相同，不是旧v4的重命名或现场转换。

来源：https://huggingface.co/Kijai/MiniMax-H3_comfy/tree/4c24460ce97e4c4b01acfe7ebcb7e01e1a86a234/loras

固定SHA256：`8e05b7b982c3aff7deb692a188c8a8d8acaeff8a12abfe1aeac822fb8ee3f0b7`；364638304字节。作者该版本原生未训练AdaLN，部署后的文件208组A/B输入输出及rank均匹配主模型；没有人为删除、截断、跳过任何权重。原pruned主模型及用户生成参数保持原值。

## 交付
- 完整图 `workflows/refine.manual.api.json` 更新配套LoRA。
- `companion/models.json` 固定来源修订、大小、SHA及明确的版本差异。
- `companion/scripts/install-models.ps1` 在正式落盘前验大小与SHA；已存在错误文件显式拒绝，不覆盖。伴随requiredFiles/inventory同步，中英文README给安装步骤。
- 已实际用该脚本部署到 `L:/AI/ComfyUI-Installs/ComfyUI (1)/ComfyUI/models/loras/`，在线节点定义已列出新模型，未启停服务。
- `install-a21/adapter-install.json` 为新包安装入口，A20不覆盖。Cargo/lock/manifest/SOURCE/WASM同步1.0.0-a21；上游与companion运行时版本保持1.1.2/a09，不因脚本修复升版。

## 定向验证
- `cargo test --lib --offline`：15 passed。
- `Build.ps1`：wasm32-wasip2 release成功。
- 部署文件SHA和208组形状实测：`a21-deployed-lora-compatibility.json`（pairs=208，problems=[]，adalnKeys=[]，runningComfyListsModel=true）。
- 唯一正式最终CLI检查：`a21-real-cli-check.log`；工具身份和时间分别见`a21-check-runtime-hashes.txt`、`a21-real-cli-check-timing.txt`。
- 本代理未提交生成、启停服务或改DB。真实软件精修及所有LoRA实际应用仍由根线程继续验收，静态shape和包检查不能替代完整生成。

## 固定产物与实际包检查

- WASM SHA256：`7e8797ac6631e7adb1255f9d0232603e6ceb098435dcfea72e0d996fda8f25e8`。
- manifest SHA256：`c97773bb2a4755e73fc7a7386c7df2857071ed2bf57a4cb34390b41cfc578142`。
- CLI SHA256：`0600a84f195c8846f598bdc20866c20a9d923bf9cb74445baac897faf69dffb2`。
- worker SHA256：`61adb11131d7018b276a8b9029a5233e2e16c4fd538684eb2a7d89f455ad7ec4`。
- 实际CLI：VALIDATED / exit 0，33605毫秒；外部计时34614毫秒。capabilities 17088毫秒、probe 15374毫秒，网络disabled；inventory/schema/signature-policy/wasm-capabilities/wasm-package-validation均通过。
- 主线程已正式安装A21，真实分析任务 `b283a6ed…` done，预览 `24c28fb3…` 已进入服务；最终精修尚待主线程反馈，不能提前记PASS。

## 离线源边界核对

旁边 `MiniMax-H3-Continuum` 的v4引用属于续写产品和其turbo节点，并非人脸精修入口，本轮不改。真正遗留人脸精修说明夹 `MiniMax-H3-FaceRefine/README.md` 指向 `MiniMax-H3` 与 `API/minimax-h3/video_minimax_h3_face_refine.json`。该图在主仓认证源、离线API及MiniMax-H3/API三处均使用ref2va pruned，未含本轮的LoRA加载节点，因此不能全局将v4替换成A21 LoRA。A21独立人脸精修包的模型配套由本包 `companion/models.json` / `install-models.ps1`交付；若后续把此独立包纳入离线发行，仅新增这个精确模型及清单，不替换其他产品v4。

## 最终真实软件链路：PASS（主线程验收）

- 分析：`b283a6ed-fd70-4d65-80d1-609feb4e3a43`。
- 跟踪预览：`24c28fb3-4542-4f3b-ab79-04479e23d02b`，done/ready，目标建国，无lost。
- 精修：`b80c024e-02a2-4b39-97fb-b3016e3a421a` done；远端 `dc9d332e-45bb-4eb0-a79c-65ee7d0cbee5`，服务149.16秒；208个patch应用，无ERROR。
- 全视频与音轨解码exit 0；1376×768、107帧、24fps、4.458秒。原节点当前版本为 `b0c5069c-5d53-4408-a0e5-d8dffcbd78bf`，原5个版本及A20结果保留。
- 主线程截图/点击工具不可用，**未完成GUI点击验收**，不把软件内核链路通过写成GUI点击PASS。

原始证据保留主仓 `logs/face-refine-acceptance-20261031/`：`a21-analysis-result.json`、`a21-preview-result.json`、`a21-final-completed-task.json`、`a21-final-service.log`、`a21-final-media-probe.json`、`a21-final-full-decode.log`、`a21-node-writeback-evidence.json`。本仓仅引用位置和验收摘要，不复制用户payload或媒体。

旧A18/A19/A20已由主线程正式卸载：changeSet `chg_face-refine-retire-old-a18-a20-20261031`，committed 3 writes，原件为主仓同目录 `a21-retire-old-apply-result.json`。本仓三个旧候选目录核对plugin/version后移入系统回收站，非永久删除，逐文件SHA审计见 `20261031-retired-candidates-recycle.json`。最新交付包只保留A21。
