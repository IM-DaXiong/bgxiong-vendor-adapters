# Z-Image零外部技术提示独立作者源码归档

当前 `author/manifest.json` 版本是 **1.0.2**（`14c9d89` 从归档叙述中的 1.0.1 抬高）。这是 0.6.1028 验收过的 `local.zeroauthor.comfy-zimage-turbo` 作者源码；2026-10-24 归入时的文字仍按 1.0.1 描述，不以那一句覆盖 manifest。没有在本说明里把 1.0.2 写成一次新的 GPU 验收。

宿主验收报告不在本公开仓库，本次未能读取产品仓原文。原 OPERATIONS 与 Invoke 脚本若出现在作者机器路径，只供追溯，不作为新机器的默认值。准确原件在 `author/workflow/source.json`。

重建时把完整author/复制到无隐式Cargo配置的可写仓外目录，并从当前安装资料取得完整vendor-adapter-v2 SDK。用其bin内CLI/worker执行AUTHORING.zh-CN.md的build/check/pack步骤，显式指定SDK与新作者目录；先准备Rust1.95.0、wasm32-wasip2与MSVC，首次显式联网取得锁定依赖，输出父目录先存在。构建后从原始workflow/source.json记录的摘要关联包身份、版本与操作。不要复制旧adapter.wasm冒充独立编译；本归档不提交构建缓存或运行包。

输入为prompt、width、height、seed、advanced.steps；输出为一张SaveImage 9原始PNG。原验收完成软件安装关联、一次真实出图、回收与原图另存，三项行为测试通过；该新目标重启续办及新视频盲验未验。公开SDK完整治理仍未完成。
