# H3 文字＋双参考图＋原音作者包（P09开发版已验收）

**P09A独立作者包/CLI及P09B开发版软件验收通过（2026-10-23）。** 新主程序已实际安装本包、关联原件，完成文字＋双图＋完整原音真实生成、成果回收、正常退出重开原号续办、播放和原片另存。SDK192文件、CLI/worker及P09A build-2执行产物保持固定；不是新NSIS安装版验收。

独立包local.example.comfy-h3-r2v-audio@1.0.0。输入为文字、两张有序原图和一份从0秒开始的完整8秒WAV，档位864×480/24fps/192帧。advanced.audioUsage必须为soundtrack_copy；原音既接入136.ref_audios.ref_audio_0生成条件，又接入130.audio最终封装，原生重采样/编码后复用，不称模型生成的输出音轨。禁止静默裁剪、循环、替换或把voice_timbre当soundtrack_copy。条件来源及图摘要见SOURCE.json。

导出后只依赖公开SDK、固定CLI/worker、Rust及PowerShell。与其他样例相同：运行New-ComfyExample.ps1建立新独立目录，再Build-ComfyExample.ps1标准build/check/pack/check；输出父目录须存在。Invoke-H3R2vAudioExample.ps1使用固定Runtime、Workspace、Endpoint、ProbeExecutable，caps/probe检查包和连接。submit需RequestFile、恰好两项有序ReferenceFiles，以及AudioFile；请求媒体ID为image-input-1/2和audio-input-1，音频原件会复制进显式workspace。SDK实际测量类型及时长，作者包在任何上传/生成前拒绝非WAV、非8秒或缺音；query只用原SubmitRequestId，不重复submit，不调用全局interrupt。

**软件消费范围：** 必须使用包含P09B通用音频消费者的新程序，原P08冻结程序不支持本包。既有audioRelPaths及referenceAudioUsages与作者声明一致，用户显式确认soundtrack_copy；两张原图顺序和完整WAV原件经受管路径授权、复制及摘要冻结，既有checkpoint继续恢复。当前音频输入限定单份WAV、最多16MiB；本包另严格要求完整8秒。缺项、数量/用途冲突、非法格式及越界路径在现有匹配行为中验证，复用有效结果。本包不证明任意图片数量、任意音频格式、音色参考、口型同步或模型联合生成音轨。

开发状态与历史失败以项目分步PLAN§16–17为准；P09A材料在E:/CodexWork/comfy-generation-p09-20261023，P09B实物在E:/CodexWork/comfy-generation-p09b-20261023-0824，运行Open-P09B-Acceptance.ps1后打开「P09B 双图与原音验收」。此入口使用固定新主程序、原UI及隔离验收数据，依赖当前开发机仓库/Node，不是用户端安装器。上述为P09当时开发入口历史，不是安装后作者依赖；P11安装资料、P10 LTX2.3与P12/P13独立作者现已分别通过。安装后直接使用本完整开发包及上级生成索引，不运行验收机专用启动脚本。


P09A实测：四项独立行为、标准build/check/pack/packed-check通过，当前可保留产物build-2。真实双图＋音频一次远端受理、独立query进程沿原request收回；原片1882853字节，H.264864×480/24fps/192帧8.000秒，AAC48kHz双声道8.000秒，完整解码通过。SDK对象与远端原片SHA892ea0b0…相同。原音和成片各解码为48kHz单声道384000样本，从0秒直接比较相关系数0.999895、相对RMS误差0.014528，未裁剪、对齐或补音；编码字节不同属原生AAC，不要求与WAV文件哈希相同。实际节点136音频条件和130原音封装双链均保留，125采样器新执行，加载器/模型/噪声等11节点缓存，不称零缓存。

首次CLI因作者mime错误在出网前被拒，build-1及失败回执保留；修正使用冻结SDK实际测量的audio/x-wav和durationMs十进制字符串，原grant进入测试fixture。7秒WAV真实反例明确拒绝。SDK和P08宿主原字节不变；P09A阶段末远程b44a76d7d无更新。以上是独立CLI记录，软件结论由下述P09B实测另行证明。

P09B实测：efa262476＋本地P09B新主程序7m22编译通过（SHA2d33d832…），复用原P08生产UI。软件内安装local.example.comfy-h3-r2v-audio@1.0.0并关联wf-7d1df36228d09248dd705bab32b6d571。素材通过标准画布模板GUI导入，音频波形播放及有序双图/原音用途确认通过；未以操作系统跨窗口拖拽或MCP直接导入冒充此结果。

唯一GUI任务50f68be6-8c5c-4db2-855b-9f9a95f51362，远端245481f9-21f9-47d7-8c54-7455b006d01f；09:03提交后关闭面板，09:04正常退出，09:04:37重开沿原号续办，09:06:52完成。submit回执1、query53，原两条视频历史保留，只增加一条本任务版本。远端、受管文件及GUI另存原片均2479513字节/SHA33bd7f28…，全片解码和播放到8秒通过；H.264864×480/24fps/192帧8秒及AAC48kHz双声道8秒，原音完整解码相关系数0.999895，未裁剪、循环或替换。

本次实际GPU文字是完整288字符的既有双图提示词。准备时自动化set_value仅改变contenteditable显示，未改变编辑状态；真实回执如实保留。随后用正常键盘输入证明新249字符可完整保存，并恢复实际提交文字；未为此重复GPU。使用自生成测试音，不声称已验证人声语义或口型。阶段末源码已合并a12d2f3d9，其剧集保存/参考图菜单更新不改P09B链，复用efa构建实跑；两版本身份分别记录，SDK192文件摘要不变。已有全仓警告/仓外旧VDN门禁失败未在本步扩修。
