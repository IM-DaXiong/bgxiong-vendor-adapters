# LTX2.3 文字＋单图＋参考音作者包（P10开发版已验收）

只依赖公开冻结SDK/CLI/worker，复用P09B通用音频消费者；不改SDK或宿主。独立身份local.example.comfy-ltx23-ia2v@1.0.0。一张有序原图和一份完整8秒WAV（从0秒，≤16MiB），768×768/24fps，原生8k+1时序193帧，名义8秒，实际播放约8.041667秒。首轮图像强度0..1可调，第二轮原生1；双轮ManualSigmas保持原图，不提供假采样步数。

advanced.audioUsage须显式voice_timbre，对应现有参考音/解码分支；输入音频在原生图中完整编码为音频latent、零噪声掩码保护，参与AV生成，输出经LTX音频VAE解码。不是原音文件直通，也不是只用音色后生成任意新声；不承诺口型/人声语义。不得选择soundtrack_copy或将CreateVideo.audio改接原文件冒充原图。原生Trim从0秒取8秒；本包事先要求完整8秒原音，不静默裁剪长音频、循环或补音。

运行Export-Ltx23Ia2vExample.ps1导出到新绝对目录；New-ComfyExample.ps1 -Project新绝对目录；Build-ComfyExample.ps1 -Project作者目录 -OutputDirectory新输出目录，父目录须存在。固定运行时/ffprobe后，用Invoke-Ltx23Ia2vExample.ps1 caps、probe；submit传request-ia2v.json、恰好一项ReferenceFiles、AudioFile；query只用原SubmitRequestId，不重复提交，不调用全局interrupt。软件另按包安装、准确原件摘要关联、显式参考音用途、真实生成、回收和正常退出原号续办验收。

来源和模型见SOURCE.json及sample-source/src/workflow-api.json；原项目API55节点保留，工作流内提示词改写开关保持false。原图/音上传原字节，原生图内尺寸预处理公开保留。素材/模型可用性与实跑结论分别记录，不以编译通过冒充软件可用。2026-10-23 P10开发版已通过，P09B原产物保持不变；不是新NSIS或独立作者验收。

独立四项行为及标准build/check/pack/packed-check、CLI caps/probe通过；首轮离线新缓存缺sha2，保留失败日志，显式复制前序同一锁定registry后通过，未改SDK/全局环境。软件使用固定P09B主程序SHA2d33d832…，包WASM02e4d66bcef378d6fd340040cb2bf3f08e97ff764513f855d3feea17c7c578fb；安装关联原件wf-55f75325044f2542042f76e18237055c，sourceRevision1，SHA b37506c3…。

唯一GUI任务46e90ec7-5f7d-4ec1-9e7b-625b91f79c37，远端c854d96b-3ba5-4a4a-b2ed-5c71b190d43a；196字符完整提示词、种子20261026、强度0.7、单图和WAV原字节、显式voice_timbre参考/解码用途核对通过。10:36:49提交，关闭面板继续回收；正常退出时原号仍running，10:38:11重开沿原号续办，10:41:54完成，约305秒。submit回执1/query67，仅新增一条本任务历史，旧两条保留。GUI播放到8.041秒并另存原片，远端/受管/另存摘要相同，完整解码退出0。

本次原片5631440字节，SHA bf7f1a884a8e42427cc4c4fe05a38c6a44e9c6f9d2367c61c70d99916b14ca8c；H.264768×768/24fps/193帧8.041667秒，原生VAE解码后的AAC48kHz双声道7.970秒，音视频流长差约71.667ms。没有补音、改播放速度或改接原音来追求等长，不承诺样本/字节级原音复用、人声语义或口型。自生成8秒测试音参与实际LTX音频编码/AV采样/解码链；六个原生常量节点缓存，不称全部零缓存。

验收实物在E:/CodexWork/comfy-generation-p10-20261023-0953，运行Open-P10-Acceptance.ps1并打开「P10 LTX2.3 图像与参考音验收」；该入口依赖本开发机仓库/Node。源码阶段末同步e4023717f，仅文档规范更新；SDK192/固定主程序/UI/CLI/worker摘要不变，有效P09基础行为结果复用。完整回执/历史失败及实际程序身份见accepted.json与项目分步PLAN§18。
