# 媒体目标选择 profile v1（media-target-selection-v1）

协议仍为 2 / WIT 2.0.0；不新增传输 operation 或 WIT 方法。复用现有 `videoFaceRefine` 能力槽与 submit/query/cancel，以及结构化 `data` + 媒体 `outputs`。

包必须声明 `requiredFeatures: ["media-target-selection-v1"]`。不支持的宿主在执行前拒绝。业务 operation ID（建议语义名 `subjects.analyze` / `subjects.preview` / `video.refine`）由**包声明**，公共 SDK 不硬编码这些字符串。

## 本目录交付

| 文件 | 用途 |
|------|------|
| `source-identity.schema.json` | 来源身份；禁止路径与显示名 |
| `analysis-receipt.schema.json` | 分析回执（可续页 shots/candidates） |
| `selection.schema.json` | 逐镜头 target/absent；未选择 ≠ absent |
| `tracking-receipt.schema.json` | 跟踪预览与丢失/未解析时段 |
| `freeze.schema.json` | 最终提交冻结 |
| `analyze|preview|refine.*.schema.json` | 三阶段 IO 形状 |
| `fixtures/receipts.json` | validator 样例 |

坐标：显示方向校正后源像素；原点左上；框半开；时间 ms 相对所选片段起点。

## 中立性裁决（A02）

1. 本地检测/跟踪选脸；2. 通用对象选择/局部视频编辑。公共形状升入本 profile；厂商节点名/模型文件名/Comfy 路径留包内。

## 生成

经 `SchemaRegistry::public` 注册（同 image-projection-v1）。暂不加入 `generate_contracts.py` wire NAMES；需要类型时走现有生成器，禁止手改生成文件。protocolVersion/WIT 不变。
