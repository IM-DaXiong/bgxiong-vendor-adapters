# H3 Continuum 图文音：独立作者样例（P13已验收）

完整开发包仅依赖随附public-kit/vendor-adapter-v2公开SDK/CLI/worker及用户ComfyUI服务。身份local.independent.comfy-h3-continuum@1.0.1，操作local.independent.comfy-h3-continuum/continuum-driving-audio。冻结工程为author；无需主仓私有源码或历史验收目录。

把整个目录复制到可写盘，准备PowerShell7、Rust1.95.0、wasm32-wasip2、MSVC原生工具。首次构建按锁联网获取依赖，只有本作者目录缓存齐备后使用-Offline。输出父目录先存在，输出目录不得存在。

```powershell
$Kit = 'D:/my-continuum-kit'
$Output = 'D:/author-build/continuum-1'
& "$Kit/Build-Continuum.ps1" -Project "$Kit/author" -OutputDirectory $Output
```

脚本执行标准build/check/pack/runtime check。runtime是严格运行包，install另含workflow-templates.json供应用安装。用户须提供已跑通的工作流原件、原生API/节点定义、模型依赖、素材及输入输出要求；作者改自己的身份、schema与图，不改SDK或宿主。

本包固定一张原图＋完整5秒WAV＋文字、512²/24fps/5秒/6步/1段。图片同时作首帧和身份参考；完整原音从0秒进入DrivingAudio生成条件，原生装配保留该原音（可能重采样/AAC编码），不是仅在CreateVideo末端拼音。本次使用合成音，不证明人声口型、跨段续接或任意长音频。图像PNG/JPEG/WebP、音频WAV，每份≤16MiB；精确合同见author/schemas/input.json。

advanced必须是宿主可消费的闭集标量；本包runName为显式单项enum/default p13-independent-software-v101-20261023，不支持任意名字。缺失或未声明字段仍拒绝，宿主不会补齐。每次测试应显式设置种子；相同存储名和相同输入可能原生复用，不承诺每次重采样。若需要新的运行名称，显式维护自己的包声明并升版，不能静默改名或删除远端存储。

Invoke-Continuum.ps1支持caps/probe/submit/query。填写真实Endpoint及本机已有ffprobe绝对路径，先caps/probe；Workspace内生成完整host-config.app.json，包含网络/预算/传输/媒体探测，不能只填endpoint。FFmpeg通过软件现有运行时入口取得，安装包不捆绑，也不回退PATH。根host-config.app.json与author/host-config.example.json为匿名样例，使用前显式替换endpoint、network.origins及ffprobe路径。

```powershell
$Endpoint = 'http://YOUR-COMFYUI-HOST:8188'
$Probe = 'D:/your-managed-runtime/ffprobe.exe'
& "$Kit/Invoke-Continuum.ps1" -Action caps -Runtime "$Output/runtime" -Workspace 'D:/author-live/continuum' -Endpoint $Endpoint -ProbeExecutable $Probe
& "$Kit/Invoke-Continuum.ps1" -Action probe -Runtime "$Output/runtime" -Workspace 'D:/author-live/continuum' -Endpoint $Endpoint -ProbeExecutable $Probe
```

软件设置→第三方适配器→SDK v2填写完整配置，安装Output/install；工作流库导入author/workflow/source.json并关联准确版本/操作。自由画布视频工作台连接一图与5秒WAV，确认完整文字、音轨复用soundtrack_copy、runName与固定档位，实际提交后检查回收/播放/原片保存。若先前失败留下能力快照，现有版本可能需要正常退出重开才刷新；不通过改宿主绕过。受理后仅查原submit requestId，不自动重提；远端历史丢失不能保证恢复，无远端取消支持。

P13标准独立构建、软件安装/关联、真实生成/回收/播放至片尾/原片另存通过。原失败后经用户授权另起一次新任务通过，不是原号恢复。详情见[最终验收与边界](FINAL-ACCEPTANCE.zh-CN.md)。SOURCE与AUTHOR-RESULT历史材料若保留，反映当时作者交付状态，不覆盖本次最终验收。
