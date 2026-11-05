# Z-Image零外部技术提示独立作者源码归档

这是0.6.1028验收过的local.zeroauthor.comfy-zimage-turbo@1.0.1作者源码原件，2026-10-24按用户要求归入主仓；author/内源码、schema、原始工作流与密封vendor-sdk字节未改。没有新增SDK/宿主接口，没有重新编译或重复GPU验收，也暂未加入sync-user-example-packs.ps1的12套安装切片清单。

真实结果见项目docs/dev/reports/comfy-authoring-v2/20261024-REPORT-0.6.1028零提示独立生图作者验收.md及同目录20261024-zero-zimage/验收原件。原OPERATIONS.txt和Invoke-ZImage.ps1保留验收时的实际路径供追溯，不把作者机器路径当作新机器的默认值；SOURCE.json中的author-input/workflow.source.json是验收输入来源名，准确原件同时包含在author/workflow/source.json。

重建时把完整author/复制到无隐式Cargo配置的可写仓外目录，并从当前安装资料取得完整vendor-adapter-v2 SDK。用其bin内CLI/worker执行AUTHORING.zh-CN.md的build/check/pack步骤，显式指定SDK与新作者目录；先准备Rust1.95.0、wasm32-wasip2与MSVC，首次显式联网取得锁定依赖，输出父目录先存在。构建后从原始workflow/source.json记录的摘要关联包身份、版本与操作。不要复制旧adapter.wasm冒充独立编译；本归档不提交构建缓存或运行包。

输入为prompt、width、height、seed、advanced.steps；输出为一张SaveImage 9原始PNG。原验收完成软件安装关联、一次真实出图、回收与原图另存，三项行为测试通过；该新目标重启续办及新视频盲验未验。公开SDK完整治理仍未完成。
