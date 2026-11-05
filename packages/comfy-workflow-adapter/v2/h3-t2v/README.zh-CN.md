# H3 文生视频独立作者样例

复用公开 SDK v2；无图片、音频或视频输入，文字同时描述画面与声音。该包声明一个代表规格：16:9原生画幅（32像素对齐后864×480）、24fps、8秒（192帧=17×11+5），可改种子和1–50采样步数，默认20步。其他尺寸/时长/帧率或参考素材显式拒绝；本例不代表H3全规格支持。声音为模型联合生成，非输入音频透传。

原件来自项目认证 video_minimax_h3_t2v.json；API图由原生ComfyUI 0.42.1的导出(API)取得，SOURCE.json保存对应摘要。仅包内知道节点、HTTP、模型与17k+5规则。SDK/宿主不含H3特判。依赖服务已有对应UNET、Qwen3VL编码器、两个VAE及节点；探针连通不等于GPU可用。

## 独立编译与打包

Windows PowerShell 7、Rust 1.95.0（含wasm32-wasip2目标）、公开SDK和匹配CLI/worker。首次构建允许按锁文件取得依赖，缓存完整后才可加-Offline。

仓库维护者先运行 Export-H3Example.ps1，传入一个全新的绝对 OutputDirectory 及已冻结的 Cli、Worker 可执行文件路径，取得包含公开 SDK、工具与 sample-source 的独立作者目录。以下命令在该导出目录运行（将路径替换为绝对路径）；接入作者无需读取软件源码：

```powershell
./New-ComfyExample.ps1 -Project E:/author-h3
./Build-ComfyExample.ps1 -Project E:/author-h3 -OutputDirectory E:/build-h3
./Invoke-H3Example.ps1 -Action caps -Runtime E:/build-h3/runtime -Workspace E:/h3-cli -Endpoint http://HOST:8188 -ProbeExecutable C:/YOUR_APP_DATA/runtimes/ffmpeg/VERSION/ffprobe.exe
./Invoke-H3Example.ps1 -Action probe -Runtime E:/build-h3/runtime -Workspace E:/h3-cli -Endpoint http://HOST:8188 -ProbeExecutable C:/YOUR_APP_DATA/runtimes/ffmpeg/VERSION/ffprobe.exe
```

公共CLI原有build/check/pack/check串联，install/才是应用安装目录。caps/probe未提交GPU；可用同一命令加-Action submit -RequestFile ./request-t2v.json提交一次；保存回执requestId，再仅以query和-SubmitRequestId原号查询。不得把超时当未受理或自动重提。SDK checkpoint在远端受理后立即写入，输出由既有媒体broker流式下载并以明确ffprobe路径读取真实宽高。SDK、CLI和worker均不修改。

## 软件安装与关联

使用已具有video.generate消费者的软件版本，设置→第三方适配器安装build-h3/install。宿主配置必须是sample-source/host-config.example.json的完整HostConfig结构，不能只填写endpointBaseUrl；替换configuration.endpointBaseUrl、network.origins和probe.executable的实际绝对路径。无鉴权服务不填密钥。probe只引用用户已有ffprobe，不绑定PATH/安装目录兜底。

导入h3-t2v.source.json，明确关联local.example.comfy-h3-t2v@1.0.0的h3-t2v操作，再注册视频能力。自由画布新视频卡片选择该模型，确认文字、864×480/8秒/24fps、种子和步数后提交。验收需真实新采样、原片/音轨/元数据、回收/播放/另存及正常退出原ID续办；目录存在或包检查通过不等于这些验收通过。

状态：P06开发版真实链路已验（2026-10-23）：独立构建、软件安装关联、一次新采样、原ID正常退出重开回收、8秒192帧/24fps/864×480、AAC双声道、播放与GUI另存原片通过。阶段末远程同步另见项目计划§13；本包不宣称新NSIS或其他输入模式通过。
