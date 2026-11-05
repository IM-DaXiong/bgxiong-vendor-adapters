# 统一开放适配标准与 SDK 开发者预览版

合同版本 2.0.0 · 安装发行 0.6.1030 · 2026-10-24。当前交付定位为 **ComfyUI 工作流作者开发包·开发者预览版**。先读 [预览版状态与使用边界](DEVELOPER-PREVIEW.zh-CN.md)，再读 [作者指南](AUTHORING.zh-CN.md)。本次SDK版本对齐只修订资料和发行元数据，SDK接口、schema、WIT和执行工具不变；主程序修复与实际验收以0.6.1030发行回执为准。

实际安装目录提供 `user-examples/vendor-adapter-v2/` 完整公开SDK，以及 `user-examples/comfy-workflow-authoring-v2/` 的任务书、工作流材料和12套自包含作者包。Windows x64下可独立编译Rust/WASM、标准check/pack，再从「设置 → 第三方适配器 → 安装适配包」安装，并在工作流详情关联准确身份、版本、操作和原件。需要宿主配置时展开「高级安装选项」，使用样例生成的完整 `host-config.app.json`。

当前接入范围是已声明输入输出原语内的图片生成、视频生成和视频编辑。图片、视频生成的已验入口以自由画布工作台及各样例README为准；视频编辑以Wan样例为准。SDK通用schema不意味着每个产品表单支持任意根字段或媒体组合。模型、节点、素材、鉴权和尺寸/时长要求由具体工作流材料说明。普通工作流能直接用现有工作流管理接入时，优先复用现有管理能力。

P01—P13的代表性编译、安装关联、真实生成和回收结论按原基线复用；0.6.1028另有新Z-Image工作流零外部技术提示独立作者闭环。该新目标包未收进本安装包，也没有据此证明新视频、所有工作流或所有操作系统。本次0.6.1030沿用这些已有证据，资料一致性及实际安装检查以本版发行回执为准，不把旧实跑写成本次新实跑。

`RELEASE-STATUS.json` 中 `scope` 明确旧G00—G06、`sdkReady=false`、`moduleIntegration`等字段属于独立SDK完整候选治理，不是当前应用功能否定清单。`developerPreview`单独记录本次有限交付；尚未完成的完整SDK验收仍保持未验。v1厂商插件原有接口继续见 `user-examples/vendor-adapter-v1/`；本预览版不承诺所有厂商/业务槽已接入v2。

软件只提供一套平台无关的接入标准。官方 API、自建服务、云端聚合服务、ComfyUI 工作流，都通过同一个适配器接口接入；“官方”只表示包的作者和发布来源，不代表专用执行通道。用户的智能体只凭公开资料制作包，软件导入后发现其能力、显示参数并执行，无须项目方为每个服务修改主程序。

本目录提供开放标准、SDK、参考宿主及独立作者工具。ComfyUI 适配、视频工作台是下游需求样本，不定义 SDK 边界。SDK制作和验证在独立workspace/参考宿主完成，不修改主程序构建、安装组件和用户数据；通过独立验收后，各模块再按自己的准备程度接入。未接入模块继续原有完整实现，已接入模块不设置失败回退或旧字段兼容；SDK_READY不等于主程序已支持v2。

阅读顺序：

1. [开放标准](OPEN-STANDARD.zh-CN.md)：责任边界、配置、能力、参数扩展、执行与版本。
2. [SDK 接口规范](SDK-CONTRACT.zh-CN.md)：包结构、调用字段、Rust/WASM 接口与生命周期。
3. [标准验证和发布条件](CONFORMANCE.zh-CN.md)：什么证据才能证明标准可独立使用。
4. [能力声明 schema](schemas/capability-contract.schema.json)、[调用 schema](schemas/operation-payload.schema.json)、[事件 schema](schemas/stream-event.schema.json)、[示例](examples/capabilities.json)。这些及同目录的信封、响应、结果、媒体、用量和 UI schema 是公开合同；纯 schema 校核不替代真实运行验收。

已有的 worker、沙箱、媒体、凭据、权限、任务、恢复和包安装系统继续作为实现基础；升级它们消费同一标准，不新建并列运行内核。标准字段不绑定平台，平台端点、鉴权报文、工作流节点、参数编码和厂商错误映射只在适配包中。

本标准保证在已支持的公共输入/输出/交互原语内，新增服务、模型、工作流、参数和业务操作通过安装适配包完成。全新的媒体类型、硬件权限或交互原语需要标准升级；不能声称未来所有未知功能均可零主程序修改。


设计校核：安装tools/requirements-design.txt后，运行 `python tools/validate_design.py`（可从任意工作目录执行脚本的实际路径）。只读校核，不发网络请求、不构建SDK、不执行模型。公开 schema 与静态能力样例用于合同校核。三个可编译 Rust/WASM 示例位于 examples/sync-api、cloud-task、workflow；真实执行、完整故障矩阵与候选就绪须分别有验收证据。

[请求/回执分派表](operation-contracts.json)固定六类传输操作；[跨领域能力样例](examples/coverage-capabilities.json)涵盖文字、文件分析、自定义能力和同模型多视频能力。标准要求完整的外部独立作者验证，设计样例不能替代它。

作者从 [AUTHORING.zh-CN.md](AUTHORING.zh-CN.md) 开始；参考宿主见 [HOST-INTEGRATION.zh-CN.md](HOST-INTEGRATION.zh-CN.md)。[独立作者任务](AUTHOR-TASK.zh-CN.md) 与 [模块接入清单](HOST-ADOPTION-CHECKLIST.zh-CN.md) 分别约束外部可用性和后续模块接入。发布候选的许可库存不授予新的所有者许可或外部分发授权。

## Native 媒体消费工具候选

协议/WIT/author SDK 2.0.0 保持不变；native tools/reference-host 2.0.1 的只读 inspect/export 候选见 [受管媒体工具](RETAINED-MEDIA.zh-CN.md)。原已验执行工具独立保留，新工具的通过范围不等于整体发行通过。
