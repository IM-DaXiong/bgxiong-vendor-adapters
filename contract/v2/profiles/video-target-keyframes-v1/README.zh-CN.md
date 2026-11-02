# 视频目标关键帧纠偏 profile v1（video-target-keyframes-v1）

协议仍为 2 / WIT 2.0.0；**不新增**传输 operation 或 WIT 方法。复用既有 submit/query/cancel、受管媒体、checkpoint 与结构化 data + 媒体 outputs。

包必须声明 
equiredFeatures: ["video-target-keyframes-v1"]。宿主不支持时在执行前明确拒绝；不得静默退回单帧选点。本 profile 与 media-target-selection-v1 **并列**：旧分析选脸合同不改名、不追加破坏性必填字段。普通选点继续走旧 profile。

## 中立语义

- **单一目标**、一个连续片段内的多纠正关键帧（正/排除点 + 至多一个矩形）。
- **不要** analysisId / candidateId；不要求先做人脸检测。
- 公共形状只描述对象选择、方向、进度事实、遮罩回执与生成冻结。
- **禁止**在公开 schema 中出现 SAM/Comfy/厂商节点名、磁盘路径、平台错误码。
- 坐标：显示方向校正后的源帧像素；框半开；时间在 SDK 输入中为**片段相对毫秒**（宿主只减一次 
angeStartMs）。

## 包声明：x-targetTracking

放在包 inputSchema 中，由 SDK schema-policy **显式允许并校验**（不得被消费者当未知字段忽略）。形状见 declaration.schema.json。

规则：

1. 宿主只按声明中的 **roles 精确 operationRef**（operationId + operationVersion + inputSchemaDigest）调用；禁止按 operationId 子串猜测 preview/nalyze。
2. 	rackingPreview 不得冒充最终生成候选模型展示。
3. 未声明的 tool / direction 不得在 UI 展示或提交。
4. centroidSummary 若存在，仅为结果摘要，**不等于**原点轨迹。
5. 参考图/提示词变更是否使跟踪失效，只看声明中的 	rackingDependencies / 
eferenceChangeInvalidatesTracking；宿主不推测模型语义。

## 本目录交付

| 文件 | 用途 |
|------|------|
| declaration.schema.json | x-targetTracking 声明闭集 |
| selection.schema.json | 纠正集合（keyframes / points / box） |
| 	racking.input.schema.json | 跟踪请求（含 direction） |
| progress.schema.json | 分方向进度事实 |
| 	racking-receipt.schema.json | 完成回执（covered/lost/uncomputed/skip + ready + opaque maskArtifact） |
| rame-page.schema.json | 可续页逐帧清单 |
| generation-freeze.schema.json | 生成冻结（绑定 receipt + mask） |
| ixtures/receipts.json | validator 样例 |

## 角色序列（宿主协商）

1. UI 草稿 → 宿主冻结 selection → 按角色调用 	rackingPreview submit  
2. 消费 progress / receipt；逐帧预览走受管媒体或 rame-page  
3. 纠正变更使 selectionDigest 变化 → 旧 receipt 立即过期 → 显式重新跟踪  
4. 确认后构造 GenerationFreeze → 按角色调用 generate，适配器读取**同一份**确认遮罩  

未跟踪仅有 selection 时保持「已选人，尚未跟踪」；不得伪造整段完成或 
eady=true。

## 生成与注册

经 validator SchemaRegistry::public 注册（同 media-target-selection-v1 / image-projection-v1）。需要类型时走既有生成器，禁止手改 generated。protocolVersion / WIT 不变。
