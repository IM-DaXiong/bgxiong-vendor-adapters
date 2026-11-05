# DeepSeek 独立文本适配包

本包仅依赖冻结的公开 SDK v2 Rust 库和锁定 registry 依赖，不依赖 MiniMax 包、应用源码或内部模型 Transport。用于检验新厂商能否通过独立包接入已有 SDK 和软件文本宿主。

声明 `text.generate`，模型为 `deepseek-flash`、`deepseek-v4-pro`，操作 `local.bgx.deepseek-text/complete`。SDK/宿主处理标准身份、schema、消息、输出与reported用量；DeepSeek的地址、Bearer、报文、thinking和错误转换只在本包。

当前操作支持普通多轮文本、缓冲回复、思考开关、独立reasoning及真实用量。`maxTokens`映射`max_tokens`（1—393216）；`thinking`映射`enabled`/`disabled`。temperature只在显式非思考模式下接受0—2，其他组合明确拒绝，不静默删除无效参数。输入原文和历史消息顺序保留。

公共计量为input/output/total tokens；缓存和reasoning细项使用本包命名空间，不冒充SDK公共计量。缺用量保持null，不估算。无自动换模型、兜底或请求重试。

官方核对2026-10-25：[Chat Completions API](https://api-docs.deepseek.com/api/create-chat-completion/)、[Thinking Mode](https://api-docs.deepseek.com/guides/thinking_mode/)。请求配置的精确baseUrl加`/chat/completions`，现有项目账号为`https://api.deepseek.com`。工具循环、多模态和流式未在此操作接通，不能从本轮文本结果推导它们已验。

构建与校验使用公开`bgx-adapter-v2 build/check/pack`，与其他独立包相同。实际账号验证使用同目录上级的通用`verify_configured_text.py`，显式指定provider-kind、model-id及本包目录；该脚本只用provider-kind查找已配置账号，不承担厂商HTTP转换。Key经受控临时凭证提供，不能放进本包。

软件从包的真实capabilities获取不透明模型选择ID，使用既有“加入普通文本模型”入口。安装时显式提供包的连接/凭证配置；没有修改SDK或宿主来识别DeepSeek。最终通过/失败以主计划和原始命令回执为准。

2026-10-25独立包1.0.0的标准构建/检查/打包、本地真实HTTP及2项Rust行为已通过；SDK193份源码、4个工具与129份宿主源码均未因本包接入而修改。项目所配置`deepseek-flash`真实请求返回401，用户指定后续人工验收，本轮已停止认证重试；不能视为云端生成成功。

人工验收时先在软件账号配置中更新有效Key，不把Key写入包或命令行。通用脚本指定本包固定候选目录、实际app-data、`--provider-kind text_deepseek_compat --model-id deepseek-flash`，先核对项目绑定的精确provider-kind；输出目录须全新。脚本读取已配置账号，经短期受控凭证调用公开SDK，不自动切换账号或模型。网络须返回真实公网地址，SDK的禁网段规则不放宽。软件安装配置不含按次`media`键；长期账号绑定入口仍属后续通用host工作，勿复用已结束租约的凭据文件路径。
