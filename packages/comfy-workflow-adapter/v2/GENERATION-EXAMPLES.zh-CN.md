# ComfyUI 生成工作流作者入口

先按实际输入选最近的样例，再让编程智能体阅读该目录的README、SOURCE、schema和作者源码。SDK是公开开发资料，不表示源码采用开源许可证；模型和节点仍由目标ComfyUI服务提供。P01–P10按各步基线通过，P11安装取件及P12/P13独立作者软件接入均已有真实验收；新发行的安装资料完整性以本次收尾验收为准。

- [Qwen文生图](qwen-t2i/README.zh-CN.md)：文字生成一张图片。
- [Qwen单图编辑](qwen-i2i/README.zh-CN.md)：一张原图与文字。
- [Qwen双图参考](qwen-multi-ref/README.zh-CN.md)：两张有序原图与文字。
- [Qwen单任务双图输出](qwen-two-output/README.zh-CN.md)：同一任务返回两张原图成果。软件批量图片另复用既有图片包和批量入口，不是新增一种SDK协议。
- [H3文生视频](h3-t2v/README.zh-CN.md)：文字和原生生成音轨。
- [LTX2.5单图视频](ltx25-i2v/README.zh-CN.md)：一张原图和文字，原生生成音轨。
- [H3双图视频](h3-r2v/README.zh-CN.md)：两张有序原图与文字。
- [H3双图与原音](h3-r2v-audio/README.zh-CN.md)：完整8秒WAV，显式soundtrack_copy，音频参与条件并复用为成片音轨。
- [LTX2.3单图与参考音](ltx23-ia2v/README.zh-CN.md)：完整8秒WAV，显式voice_timbre参考/解码分支；输出不是原音直通，视频8.041667秒与音轨7.970秒是已验原生结果。

安装目录中的每个上述子目录是一套由仓内唯一源导出的独立作者开发包，含完整相同SDK、固定CLI/worker、sample-source、New-ComfyExample.ps1、Build-ComfyExample.ps1和对应Invoke脚本。保留各包自包含布局，避免作者依赖宿主源码或隐含仓库路径。仓内开发者先运行各目录Export脚本导出；安装后的作者无需再运行仓内Export脚本。

1. 把选中的**完整子目录**复制到可写盘，保留vendor-adapter-v2，不要只复制src或在安装目录编译。准备PowerShell7、SDK固定的Rust1.95.0及wasm32-wasip2目标，按所选README核对目标服务节点、模型、原件与输入限制。
2. 运行该副本的New-ComfyExample.ps1 -Project <新的绝对作者目录>，再运行Build-ComfyExample.ps1 -Project <作者目录> -OutputDirectory <新的绝对输出目录>。输出父目录须先存在；默认允许按锁定版本取得依赖。**仅在该新作者目录的.bgx-sdk/cargo-home已显式准备好锁定依赖时使用-Offline**；空缓存离线失败不是源码编译成功。
3. 按所选Invoke脚本用公开CLI进行caps/probe。需要真实存在的ffprobe绝对路径：应用可通过现有本地引擎入口下载FFmpeg运行时，再使用其托管目录中的ffprobe.exe；安装包不内置FFmpeg，也不回退PATH。query沿原SubmitRequestId，失败或受理未知不自动重提。
4. 软件内设置→第三方适配器→高级安装选项，填写完整host-config.app.json配置，再点安装适配包，安装本次build/install。在工作流库导入该包附带的准确.source.json，在详情关联该身份/版本/操作；随后在相应工作台选择该工作流，确认图像顺序、音频用途和公开可调字段，再提交生成。安装/关联本身不能代替真实成果回收和播放验收。

接入用户自己的图时，交给智能体[作者任务书](AGENT-TASK.zh-CN.md)、用户已跑通的原件、节点定义、模型依赖及输入/输出需求。改作者工程身份、schema、图和相应行为，不修改冻结SDK/宿主。自定义可调标量放在advanced闭集；媒体按已声明的原图/原音槽。超出当前宿主可表达输入时报告具体缺口，不借字段改名、静默换用途、默认填充、裁剪、循环或厂商特判绕过。新身份/版本和新原件摘要须重新明确关联；旧任务继续沿原包checkpoint恢复。

这些是已验证的窄样例，不承诺任意数量/格式/模型/鉴权组合。当前音频消费者只接受WAV、每份≤16MiB，各包另有固定数量及时长要求（如8秒或Continuum5秒）。精确尺寸、帧数、时长、音轨语义、缓存及未验事项以各README/SOURCE为准。P12/P13独立作者接入已通过，但得到过目标材料与诊断提示，不称零提示盲验；P13网络失败后获授权的新任务成功，不称原号恢复。

- [FLUX2独立双图参考作者](flux2-multi-ref-independent/README.zh-CN.md)：P12，两图＋文字、单PNG，含完整冻结SDK。
- [H3 Continuum独立图文音作者](h3-continuum-independent/README.zh-CN.md)：P13，一图＋文字＋完整5秒WAV，单段，含完整冻结SDK。

视频扩展参数须为宿主可消费的带默认值闭集标量；字符串扩展按样例显式enum/default声明，并保留required拒绝缺项。通用SDK包检查通过不等于当前表单能表达任意字段。需要新交互、媒体或协议时报告具体缺口，不静默补值。P12/P13仅代表已验档位，不能承诺任意图自动接入。

本次开发者预览安装发行版本为0.6.1029，基于主仓f4e88e3d4与已有本地验收成果，SDK合同/源码业务及固定CLI/worker不变。P01—P13及0.6.1028零提示新生图作者结果按原基线复用，本轮仅更新资料/发行状态、标准打包并核对实际安装资料。完整边界见 [预览版说明](../../../contract/v2/DEVELOPER-PREVIEW.zh-CN.md)。不称全SDK或全仓质量全部通过；不附工具链、模型和已下载运行时。
