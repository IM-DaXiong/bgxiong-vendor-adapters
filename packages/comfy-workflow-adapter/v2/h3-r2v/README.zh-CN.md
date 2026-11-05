# H3 双参考图生视频作者样例（P08）

独立包 `local.example.comfy-h3-r2v@1.0.0`，模型 `minimax-h3-r2v`，操作 `local.example.comfy-h3-r2v/h3-r2v`。复用冻结的公开 SDK；模型、HTTP和工作流节点全部在包内。开发者需要 Rust（含 wasm32-wasip2）、PowerShell7、可访问的ComfyUI及原图，准备好SOURCE.json列出的节点/模型。

先从本仓用 Export-H3R2vExample.ps1 导出公开作者资料，提供现有CLI和worker绝对路径。导出资料脱离仓库后执行：

```powershell
./New-ComfyExample.ps1 -Project E:/author/h3-r2v
./Build-ComfyExample.ps1 -Project E:/author/h3-r2v -OutputDirectory E:/author/build1
./Invoke-H3R2vExample.ps1 -Action caps -Runtime E:/author/build1/runtime -Workspace E:/author/workspace -Endpoint http://YOUR-COMFY-HOST:8188 -ProbeExecutable C:/YOUR-FFPROBE/ffprobe.exe
```

输出父目录须已存在，Project/OutputDirectory/首次Workspace须为新目录。Build使用SDK标准build/check/pack及packed-check；不绕过schema/能力/签名策略。Invoke caps生成完整host-config.app.json，软件安装时填写该完整JSON，并核对端点、network.origins及ffprobe路径；sample-source/host-config.example.json提供完整模板。只填configuration不足以安装。

在软件安装build1/install，工作流库导入h3-r2v.source.json，显式关联该video.generate操作；自由画布选择“多图参考生视频”。接入两张不同原图，展示顺序就是Picture1/Picture2，文字用`<Picture 1>`/`<Picture 2>`引用。它们不表示首尾帧。工作台每张原图上限16MiB，超限明确拒绝，不自动降采样；模型图内部ref_image_size=match仍按原生规则编码图像。固定16:9/864×480/24fps/8秒，192=17×11+5帧，advanced.steps可调1..50，默认20。联合生成音轨，无输入音频。

CLI submit提供request-r2v.json和恰好两项有序ReferenceFiles绝对路径；请求mediaId分别为image-input-1/2。只submit一次，之后query使用原SubmitRequestId。受理即checkpoint；关闭面板/退出后原号续办，不重提；取消不支持，不调用全局interrupt。缺图、超量、非法时序与首尾帧/音频/视频字段明确拒绝。

SOURCE.json记录认证原件及原生API导出摘要。P08开发版已完成独立编译、软件安装关联、两原件有序上传/生成条件、真实出片、正常退出同号续办、完整播放和原片另存；回收只新增一条历史，旧版本保留。实际视频864×480/24fps/192帧/8.000秒，AAC32kHz双声道8.000秒；详细身份/失败历史见项目分步计划§15。仅此两图代表档位，未宣称输入音频/视频、全部分辨率、独立作者或新NSIS通过。
