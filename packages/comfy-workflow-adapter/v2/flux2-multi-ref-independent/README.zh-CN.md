# FLUX2 双图参考：独立作者样例（P12已验收）

此目录是完整开发包；不需要主仓或宿主私有实现。SDK/CLI/worker在vendor-adapter-v2，冻结工程在flux2-author。将整个目录复制到可写盘，准备PowerShell7、Rust1.95.0、wasm32-wasip2和MSVC工具。首次下载锁定依赖需要网络；仅自己的Cargo缓存齐备后使用-Offline。

```powershell
$Kit = 'D:/my-flux2-kit' # 本完整目录的可写副本
$Output = 'D:/author-build/flux2-1' # 父目录先存在，输出目录尚不存在
& "$Kit/Build-Flux2.ps1" -Project "$Kit/flux2-author" -OutputDirectory $Output
```

构建顺序为公开SDK build/check/pack/严格runtime check。runtime是标准运行包；install另有workflow-templates.json供软件安装，不将install当严格runtime根检查。不要复制旧WASM/target冒充编译。

提供用户已跑通的目标原件、原生API、节点定义、模型依赖、输入输出与参数需求给编程智能体，修改自己的包身份、schema和图，不改SDK/宿主。样例身份local.independent.flux2-ordered-reference@1.0.0（以manifest为准）。使用前让服务提供者确认节点和权重齐备；安装包不附模型。实际支持字段、数量、格式与范围以flux2-author/schemas/input.json为准。

本代表实测为两张有序原图＋文字、1024×1024、20步/guidance4/Turbo关闭。每图≤16MiB，不能拼图替代、静默丢图或降采样。其它档位/Turbo没有新增GPU矩阵。参考图是有序条件，不是首尾帧。

运行Invoke-Flux2.ps1的caps/probe可生成Workspace内完整host-config.app.json，填写真实Endpoint，按[CLI参数说明](README.md)提交或查询；连接成功不证明模型/GPU就绪。软件设置→第三方适配器→SDK v2填写完整配置，安装Output/install；工作流库导入flux2-author/workflow/source.json，明确关联准确身份/版本/操作。图生图工作台选择此工作流、两图顺序与提示词，显式选择1:1并确认1024×1024，不能保留自动尺寸。随后真实提交并检查历史、预览和原图保存。

P12软件真实出图、自动回收、预览及原图另存已通过；详情见[最终验收与边界](FINAL-ACCEPTANCE.zh-CN.md)。独立作者使用安装公开资料并获得原生图/诊断提示，不称零提示盲验。原测试失败和来源记录保留，不冒充任意工作流一键导入。旧README.md为原作者命令记录，其待验语句以本页最终结论为准。
