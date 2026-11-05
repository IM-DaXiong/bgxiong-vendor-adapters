# LTX 2.5 单图生视频作者样例（P07）

仅使用公开SDK v2，不需宿主源码。一个必填原图＋完整提示词→一个带生成音轨MP4。原图PNG/JPEG/WebP字节不重编码上传；软件单张原图上限16MiB，超限明确拒绝，不走自动降采样；工作流自身的预处理保持原样。缺图/超量/非法档位在网络前拒绝，不降为文生视频。

## 独立构建

先在仓库执行Export-Ltx25Example.ps1 -OutputDirectory <绝对新目录> -Cli <固定CLI绝对路径> -Worker <固定worker绝对路径>。导出目录包含完整冻结SDK与工具。然后：

```powershell
./New-ComfyExample.ps1 -Project E:/author/ltx25
./Build-ComfyExample.ps1 -Project E:/author/ltx25 -OutputDirectory E:/author/build1
```

首次构建需联网缓存Rust依赖；目标wasm32-wasip2。输出父目录需存在，输出目录本身必须不存在。build/check/pack/packed-check均须成功。

## 配置、安装与关联

Invoke-Ltx25Example.ps1的caps/probe共用Runtime、Workspace、Endpoint、ProbeExecutable参数。ffprobe必须是已存在的绝对路径，不查PATH。Endpoint示例http://192.168.x.x:8188；匿名局域网不填凭证。caps后Workspace/host-config.app.json是完整匿名HostConfig，可用于软件第三方适配器配置；不能只粘贴endpointBaseUrl。

在软件安装build1/install，工作流库导入ltx25-i2v.source.json，再显式关联该video.generate操作。自由画布视频工作台选该工作流，模式仅“单图生视频”，选一张真实原图。固定1:1/768×768/24fps/名义8秒，原生length=秒数×fps+1=193（8k+1）帧，实际播放约8.041667秒；不裁尾帧、不仅改播放fps。高级参数可调首轮原图强度0..1，第二轮强度原生1。两轮固定sigma采样；提示词增强原生关闭，烘焙负面文本保留。

CLI submit另需RequestFile=request-i2v.json及ReferenceFile=绝对原图路径。只submit一次，后续query只给原SubmitRequestId。受理checkpoint后关闭面板/退出可沿原号回收，不重提。取消不支持，不调用全局interrupt。

工作流/原生API来源和摘要见SOURCE.json；未宣称多参考图、音频输入、全部LTX版本、其他档位、独立作者验收或新NSIS已验。P07实跑结果以项目计划§14为准。
