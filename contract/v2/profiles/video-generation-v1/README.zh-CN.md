# video-generation-v1：视频生成公开双向参数合同

本目录是 `video.generate` 的公开消费约定：适配器如何声明工作台参数，以及用户确认后提交哪些标准字段。合同版本仍为 V2 / WIT 2.0.0；本目录**不**新增运行信封、WIT 字段、组合解释器或 SDK 执行版本。

权威形状：

- 能力与操作：[capability-contract.schema.json](../../schemas/capability-contract.schema.json)
- 本示例输入：[`input.example.schema.json`](input.example.schema.json)
- UI 控件：[ui-schema.schema.json](../../schemas/ui-schema.schema.json)
- 媒体引用字节：既有受控媒体通道；input 内只放 `mediaId`

SDK 只做：公开字段合同、读取声明、透传标准值、既有执行通信。SDK **不懂**平台业务，不判断厂商尺寸组合是否合法，不计算平台帧格，不替适配器修参数。平台限制、节点图、URL、错误码换算全部留在仓外适配器。

## 1 双向通信一句话

1. **适配器 → SDK → 工作台**：用 `inputSchema` / `uiSchema` / `mediaSlots` / `execution` / `outputSlots` 声明支持的参数、类型、单位、默认值、枚举或范围、是否固定、素材角色与数量。
2. **用户确认 → SDK → 适配器**：传递当前所选操作身份、模式、规格、提示词、素材与其余生成参数；固定参数传声明值。
3. **适配器 → 云端**：标准字段映射成平台字段并执行业务校验。
4. **适配器 → SDK → 软件**：沿既有执行合同回传受理、进度、错误与视频成果。

未声明的字段不视为支持，不能用默认值冒充范围。声明之外的组合规则由适配器判断并显式报错；用户合法选择不被默认覆盖；错误不能静默换模型、改参数或重提任务。

## 2 字段逐项约定（实现时每项注明支持/不支持）

下列名称、类型与单位是公开 profile 的统一合同。宿主可按声明给出选项、默认值与基础类型/枚举/范围提示；**不能**只给作者一个自由 JSON 让其猜名称。

| 字段 | 单位/形状 | 声明方式 | 提交规则 |
| --- | --- | --- | --- |
| 模型/操作身份 | 执行信封 | `modelId`、`operationId`、`operationVersion`、包版本、`inputSchemaDigest` | 不把显示名当 ID；切操作携带完整身份 |
| `inputMode` | 字符串 | 该 operation 的 inputSchema 用 `const` 固定一种模式 | 提交当前所选操作对应的模式；不按图片数量推断 |
| `prompt` | 字符串 | `type:string`；通常 required | 用户确认后的完整文字；保留换行与素材引用文本；不截断、不擅自优化 |
| `negativePrompt` | 字符串 | 省略=不支持；声明且非 required=可选 | 支持时才显示并提交；显式清空传 `""`；不支持则不发送 |
| `durationSeconds` | 秒（整数） | `const` / `enum` / `minimum`+`maximum`(+`multipleOf`)；`default` 仅初始化 | 固定值控件锁定也须进入请求；本轮消费者沿整数秒，不宣传小数秒已接通；不由 SDK 算厂商帧数 |
| `fps` | 帧/秒 | 同上 | 传用户所选或声明固定值；不借播放 fps 改写生成帧率 |
| `aspectRatio` | 字符串 | 包提供的枚举或固定值 | 合法声明（含 `9:5`）不得被本地常见比例白名单过滤；与宽高是否匹配交适配器 |
| `width` / `height` | 像素 | 固定/枚举/范围与步长 | 原样发送解析或通用输入得到的像素值；不按平台公式生成尺寸 |
| `resolution` | 档位 ID | 命名档位枚举 | 提交包提供的档位 ID；显示 label 不是请求值 |
| `resolutionValue` | `{kind:"megapixels",value}` | 仅声明支持时启用 | 单位 MP；与像素尺寸、命名档位互斥 |
| `seed` | 整数 | 类型、默认、范围 | 传用户值；不新增 SDK 随机化/截断；超精确表示范围显式结构错误 |
| 参考素材 | `mediaId` 数组 | `mediaSlots`：`role`、`inputPointer`、`mediaKind`、`minCount`/`maxCount` | 首帧/尾帧/有序参考图/音/视频；无素材、移除、重排以当前工作台为准；原件走受控媒体通道 |
| `advanced.audioUsage` | 字符串枚举 | 有参考音槽时声明用途闭集 | 与「输出含音频」无关 |
| `advanced.*` 生成音频及其他选项 | 标量闭集 | `additionalProperties:false`；每项带 `default` | 工作台有值才传；布尔 `false`、数字 `0`、空串、空数组均合法 |
| 宿主 `vendorFeatureDraft` | 草稿 | 无独立 SDK 字段 | **必须**落入已声明的 `advanced`（或其他已声明标量）；禁止静默丢弃，禁止再另发一份重复的 `vendorFeatures` |

### 2.1 尺寸表示三选一

像素尺寸（`width`/`height`）、命名档位（`resolution`）、MP（`resolutionValue`）是不同表示。包必须明确只接收其中一种；一次请求不能混入另一种表示的旧草稿。切换表示时清除另外两路旧值。

本目录 [`input.example.schema.json`](input.example.schema.json) 选用**像素尺寸**；`$defs.NamedResolutionExample` 与 `$defs.MegapixelResolutionExample` 仅作另两种声明样式的中立示意，不是本示例根字段。

### 2.2 固定 / 枚举 / 范围

- **固定**：`const`（可同时给 `default` 供初始化）。固定值即使控件只读，确认请求仍须带该值。
- **枚举**：`enum` + `default`。
- **范围**：`minimum` / `maximum` / 可选 `multipleOf` + `default`。
- `default` 只用于工作台初始化；正式提交不靠服务端补默认、不静默删字段。

示例文件覆盖：`fps` 固定、`durationSeconds` 枚举、`width`/`height`/`seed` 范围、可选 `negativePrompt`。

### 2.3 媒体角色（不得用数量猜用途）

`mediaSlots` 每项必须明确角色。推荐角色名（可按包扩展，但语义须可读）：

| role | mediaKind | 典型 inputPointer | 含义 |
| --- | --- | --- | --- |
| `firstFrame` | `image` | `/firstFrame` | 首帧 |
| `lastFrame` | `image` | `/lastFrame` | 尾帧 |
| `referenceImages` | `image` | `/referenceImages` | 有序参考图 |
| `referenceAudios` | `audio` | `/referenceAudios` | 有序参考音 |
| `referenceVideos` | `video` | `/referenceVideos` | 有序参考视频 |

input 中对应字段是 `{mediaId}` 对象数组；字节不进 JSON。一个 operation 一种 `inputMode`；同一模型可声明多个 operation。当前宿主消费者可能尚未接通全部模式/媒体种类——**公开字段 ≠ 全部已实现**；未接通的不得注册为可用，也不能影响软件原生已有模式。

### 2.4 执行与输出

继续使用既有 `execution` / `outputSlots`。不能宣传未实现的同步/异步/取消/续办/输出形式。示例操作常见异步单视频输出：`outputSlots=[{role,kind:video,minCount:1,maxCount:1}]`。

### 2.5 软件自有字段不进云端生成参数

节点/工作区 ID、历史、保存位置、窗口布局、素材记忆、确认状态、批量数量、本地编排与成片后处理由软件 owner 管理。传给适配器的是确认后的最终 `prompt` 与 §2 标准字段，不是全部 UI 状态；提示词 Skill ID 不交给适配器理解。

## 3 适配器应提供什么 / 会收到什么

### 3.1 capabilities 声明片段（中立）

```json
{
  "schemaVersion": 2,
  "localId": "neutral-video",
  "capabilityKey": "video.generate",
  "transportSlot": "video",
  "modelIds": ["neutral-model"],
  "operations": [
    {
      "id": "neutral/first-last",
      "version": 1,
      "label": "首尾帧示例",
      "inputSchema": {"id": "input", "digest": "<sha256 of schemaJson bytes>"},
      "outputSchema": {"id": "output", "digest": "<sha256>"},
      "resumeSchema": {"id": "resume", "digest": "<sha256>"},
      "uiSchema": {
        "controls": [
          {"fieldPointer": "/prompt", "widget": "multiline", "label": "提示词", "description": "", "group": "输入"},
          {"fieldPointer": "/negativePrompt", "widget": "multiline", "label": "负面提示词", "description": "可清空为空串", "group": "输入"},
          {"fieldPointer": "/durationSeconds", "widget": "select", "label": "时长（秒）", "description": "", "group": "规格"},
          {"fieldPointer": "/fps", "widget": "integer", "label": "帧率", "description": "本操作固定 24", "group": "规格"},
          {"fieldPointer": "/aspectRatio", "widget": "select", "label": "画幅", "description": "", "group": "规格"},
          {"fieldPointer": "/width", "widget": "integer", "label": "宽", "description": "", "group": "规格"},
          {"fieldPointer": "/height", "widget": "integer", "label": "高", "description": "", "group": "规格"},
          {"fieldPointer": "/seed", "widget": "integer", "label": "种子", "description": "", "group": "参数"},
          {"fieldPointer": "/firstFrame", "widget": "media", "label": "首帧", "description": "", "group": "素材"},
          {"fieldPointer": "/lastFrame", "widget": "media", "label": "尾帧", "description": "", "group": "素材"},
          {"fieldPointer": "/referenceAudios", "widget": "media", "label": "参考音", "description": "", "group": "素材"},
          {"fieldPointer": "/advanced/audioUsage", "widget": "select", "label": "音频用途", "description": "", "group": "参数"},
          {"fieldPointer": "/advanced/generateAudio", "widget": "toggle", "label": "生成音频", "description": "", "group": "参数"},
          {"fieldPointer": "/advanced/steps", "widget": "integer", "label": "步数", "description": "", "group": "参数"}
        ]
      },
      "mediaSlots": [
        {"role": "firstFrame", "inputPointer": "/firstFrame", "mediaKind": "image", "minCount": 1, "maxCount": 1},
        {"role": "lastFrame", "inputPointer": "/lastFrame", "mediaKind": "image", "minCount": 1, "maxCount": 1},
        {"role": "referenceAudios", "inputPointer": "/referenceAudios", "mediaKind": "audio", "minCount": 0, "maxCount": 2}
      ],
      "execution": {
        "mode": "asynchronous",
        "streaming": false,
        "cancellation": "unsupported",
        "resumable": true,
        "remoteIdempotency": false
      },
      "outputSlots": [
        {"role": "video", "kind": "video", "minCount": 1, "maxCount": 1}
      ]
    }
  ]
}
```

`schemas[].schemaJson` 放入与 [`input.example.schema.json`](input.example.schema.json) 等价的包内输入 schema（或作者按支持面裁剪后的子集），并用其原始 UTF-8 字节计算 digest。

### 3.2 示例提交 input（用户确认后）

```json
{
  "inputMode": "first_last_frame",
  "prompt": "A quiet street at dusk.\nKeep the same character.",
  "negativePrompt": "",
  "durationSeconds": 5,
  "fps": 24,
  "aspectRatio": "9:5",
  "width": 1440,
  "height": 800,
  "seed": 42,
  "firstFrame": [{"mediaId": "image-input-1"}],
  "lastFrame": [{"mediaId": "image-input-2"}],
  "referenceAudios": [{"mediaId": "audio-input-1"}],
  "advanced": {
    "audioUsage": "voice_timbre",
    "generateAudio": false,
    "steps": 20
  }
}
```

校验思路（不新增工具）：

1. 用既有 SDK SchemaRegistry / 原生 JSON Schema 引擎登记包内 schema（digest、本地 `$ref`、schema-policy 关键词）。
2. 对正式 submit 的 `input` 做严格校验：缺必填、额外字段、枚举/范围错误均拒绝；不补默认、不删未知字段。
3. `mediaSlots` 与 input 数组字段、租约 `mediaId` 一致；execution/outputSlots 与能力声明一致。

## 4 与作者指南 / 宿主指南的关系

- 通用 init/build/check/pack：[AUTHORING.zh-CN.md](../../AUTHORING.zh-CN.md)
- 宿主装配与严格输入：[HOST-INTEGRATION.zh-CN.md](../../HOST-INTEGRATION.zh-CN.md)
- 预览范围与边界：[DEVELOPER-PREVIEW.zh-CN.md](../../DEVELOPER-PREVIEW.zh-CN.md)

独立作者仅凭本公开目录与上述公开 SDK 资料，即可知道应声明哪些能力信息、submit 会收到哪些标准值；平台映射留在作者包内。
