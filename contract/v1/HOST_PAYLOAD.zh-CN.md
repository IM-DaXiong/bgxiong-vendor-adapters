# 宿主 ↔ 插件字段契约（Vendor Adapter Protocol v1）

> **本仓对外开发契约（给人读的字段辞典）**  
> 第三方适配器作者实现插件前**必读**。  
> Wire 字段名一律 **camelCase**。  
> 闭集（operations、capabilitySlots、errorCodes 等）见 `protocol/vendor-adapter-protocol.json`（Rust SDK 内嵌该文件）。宿主信任路径与 providerKinds 仍在 `packages/app-contracts/v1/vendor-adapter.json`。  
> English edition: [`HOST_PAYLOAD.en.md`](HOST_PAYLOAD.en.md)

## 权威地图

| 角色 | 路径 |
|------|------|
| **本文（中文字段辞典）** | `packages/vendor-adapter-contract/v1/HOST_PAYLOAD.zh-CN.md` |
| 英文版 | `HOST_PAYLOAD.en.md` |
| WIT 世界与 Host 导入 | `wit/vendor-adapter.wit` |
| 信封与回包辅助类型 | `sdk/rust`（`Invocation`、`Response`、`SubmitResult`、`QueryResult`、`Output`） |
| Manifest / 运行时能力 Schema | `schemas/manifest.schema.json`、`schemas/runtime-caps.schema.json` |
| 宿主实现对照（防漂移） | `src-tauri/src/vendor_adapter_gateway/bridge/*_provider.rs` |
| 能力投影 | `src-tauri/src/vendor_adapter_gateway/bridge/caps_adapter.rs` |
| RCD 校验 | `src-tauri/src/vendor_adapter_gateway/domain/runtime_caps.rs` |

`examples/` **只是标本**，不是字段标准。标准 = 本文（或英文版）+ WIT + schemas + SDK 类型。

---

## 0. 设计铁律

1. 用户在 UI 选什么通道就跑什么；禁止静默换厂牌或降级。
2. 宿主不读厂商工作流 JSON。可调参数必须在 RCD 声明，并真正写入厂商请求（回传 `appliedParams`）。
3. 大二进制优先 `host-media` handle；勿默认长期依赖巨型 Base64。
4. Guest 禁止导入 WASI socket / filesystem / cli；只能用 WIT `host-*`。
5. 每个响应只能有 `dataJson` 或 `error` 之一。
6. 异步任务的 `vendorTaskId` 必须是真实厂商任务号（轮询与重启恢复依赖它）；禁止编造。
7. 用户可见 toast 由宿主按 `error.code` 映射；不要把 serde / 堆栈塞进给用户看的 `message`。

---

## 1. 调用模型

每次宿主调用插件导出的 `invoke`，传入 **Invocation 信封**。  
业务字段在 **`payloadJson`** 里：它是一个 **字符串**，内容为 JSON 对象。

```
Host                                      Guest
----                                      -----
Invocation {
  protocolVersion,
  operation,           // capabilities|probe|upload|submit|query|cancel
  profileId,
  capabilitySlot,      // image|video|text|...
  requestId,
  budgetMs,
  payloadJson,         // 业务 JSON（字符串化）
  bindingId,
  negotiatedVersion?,
  enabledFeatures?[]
}
                 -- invoke -->
                                          Response {
                                            protocolVersion,
                                            operation,
                                            dataJson?  或  error?   // 二者择一
                                          }
```

出网与媒体/凭证只走 WIT import：`host-http`、`host-media`、`host-credential`、`host-state`、`host-log`。

---

## 2. Invocation 信封（Host → Guest）

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `protocolVersion` | `u32` | 协议主版本；响应须回显宿主主版本（SDK：`protocol_host_major()`） |
| `operation` | `string` | 操作名；闭集见契约 `operations` |
| `profileId` | `string` | 宿主侧配置 / 绑定 id |
| `capabilitySlot` | `string` | 能力槽；闭集见 `capabilitySlots` |
| `requestId` | `string` | 本次调用关联 id（常与 AI 任务相关） |
| `budgetMs` | `u64` | 剩余执行预算（毫秒，单调剩余时间，不是墙上时钟截止时刻） |
| `payloadJson` | `string` | **字符串化**的业务 JSON；按 `operation` × `capabilitySlot` 解释（见第 3 节） |
| `bindingId` | `string` | 本次调用的宿主绑定 id（不是 `ai_tasks.id`；任务 id 只留在宿主侧） |
| `negotiatedVersion` | `u32?` | 协商版本（可选） |
| `enabledFeatures` | `string[]?` | 已启用可选特性（可选；默认空） |

SDK 类型：`Invocation`。业务字段用 `inv.payload::<T>()` 反序列化。

---

## 3. `payloadJson` 业务载荷（Host → Guest）

下列字段是 **当前桌面宿主 bridge 实际下发** 的标准。  
未列出的键不要依赖；未知键应忽略以保持向前兼容。  
缺省键或 JSON `null` 表示「未提供」。

### 3.1 通用操作

#### `capabilities`

通常为 `{}`。要求 guest 返回符合 `schemas/runtime-caps.schema.json` 的运行时能力文档（见第 5 节）。

#### `probe`

通常为 `{}`。连通性 / 探活。

**导入 L3**（空 payload、无凭证）不是厂商探活：guest 必须成功且 **禁止** `host-http`。仅当宿主已注入非空顶层 **`endpointBaseUrl`** 时才做出网探活。

当 manifest 声明 `userConfigurable` 时，宿主把适配器凭证 `baseUrl` 写入顶层 **`endpointBaseUrl`**（`invoke_adapter_operation` 的 submit/query/probe，以及重启回收 query）。这是通用字段，不是某一家厂商 API。导入与设置页「探测就绪」不注入该字段。

#### `upload`

视槽位与媒体管线而定。大文件优先 `host-media`，不要默认整包塞进 `payloadJson`。

#### `cancel`

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `vendorTaskId` | `string` | 要取消的厂商任务号（若该槽实现了 cancel） |

若不支持 cancel，返回契约错误码（如 `adapterCapabilityDenied`），禁止 trap 崩溃。

---

### 3.2 按能力槽的 `submit` / `query`

#### 槽 `video`（以及视频管线路由的 `keyframe`）

`keyframe` 与 `video` **共用**同一套 submit/query 字段；用信封里的 `capabilitySlot` 区分。

**submit**

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `prompt` | `string` | 用户提示词 |
| `model` | `string` | 厂商模型 / 工作流标识——`adapter:{pluginId}:` **之后**的那段 |
| `negativePrompt` | `string?` | 负面提示词 |
| `resolution` | `string?` | 分辨率或画幅（如 `1920x1080`）；与 RCD `resolution` / `aspect` 对应 |
| `fps` | `number?` | 帧率 |
| `durationSeconds` | `number?` | 时长（秒） |
| `outputFormat` | `string?` | 期望输出封装 / 格式 |
| `startFrameB64` | `string?` | 首帧图 Base64（可能很大；生产路径优先 media handle）。Guest 上传时须按魔数定 MIME/扩展名，未知格式显式失败。 |
| `endFrameB64` | `string?` | 尾帧图 Base64（同上，禁止一律写成 PNG） |
| `referenceImagesB64` | `string[]?` | 参考图 Base64 列表 |
| `extra` | `object?` | 宿主附加上下文；**不是**稳定厂商 API，只读已文档化的键 |
| `endpointBaseUrl` | `string?` | 凭证 `baseUrl`（`userConfigurable`）。Guest 用它拼厂商 HTTP |

**query**

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `vendorTaskId` | `string` | submit 返回的厂商任务号 |
| `model` | `string?` | 与 submit 相同的模型标识 |
| `endpointBaseUrl` | `string?` | 同 submit：凭证 origin |

---

#### 槽 `image` / `imageToImage`

字段集相同。有参考图时槽多为 `imageToImage`，否则多为 `image`。

**submit**

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `prompt` | `string` | 提示词 |
| `model` | `string` | 厂商模型标识 |
| `size` | `string?` | 尺寸（如 `1920x1088`） |
| `negativePrompt` | `string?` | 负面提示词 |
| `responseFormat` | `string?` | 厂商响应格式偏好 |
| `watermark` | `bool?` | 是否水印等 |
| `referenceImagesB64` | `string[]?` | 参考图；`imageToImage` 通常非空 |
| `extra` | `object?` | 附加上下文 |

**图像尺寸字段映射（一人一义）**

| 层 | 字段 | 含义 |
|---|---|---|
| submit `payloadJson` | `size` | 工作台所选清晰度，格式 `WxH` |
| RCD 槽参数 | `resolution` | `enum` 档位；`options[].value` 与 `size` 同值。**不要**再声明独立 `aspect` 参数；画幅是 option 元数据 `aspectRatio`（`16:9` / `9:16` / `1:1` / `4:3` / `3:4`） |
| submit 回包 | `appliedParams.resolution` | 实际写入厂商请求的 `WxH`，必须与所选 `size` 一致 |

图像槽 **禁止** `resolution.mode=range`（一维标量无法驱动宽高两列）。`fixed` 时宿主不下发 `size`；guest 若仍收到 `size` 应 `adapterInvalidRequest`。无 RCD / 未协商时宿主按工作流原生尺寸处理，同样不下发 `size`。

**query**

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `vendorTaskId` | `string` | 厂商任务号 |
| `model` | `string?` | 模型标识 |

---

#### 槽 `text`

**submit** 为**同步等待**：必须直接返回 `kind: "completed"`。宿主**不**轮询。

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `model` | `string` | 厂商模型标识 |
| `messages` | `{role, content}[]` | 对话消息 |
| `temperature` | `number?` | 采样温度 |
| `responseFormatJson` | `bool?` | 是否要求 JSON 形态输出 |
| `maxTokens` | `number?` | 最大生成 token |
| `thinking` | `bool?` | 是否启用思考链（若模型支持） |

当前文本槽不走 `query`。

---

#### 槽 `speech`

**submit**

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `text` | `string` | 待合成文本 |
| `voiceId` | `string?` | 音色 id |
| `model` | `string?` | 模型标识 |
| `mode` | `string?` | 合成模式（宿主枚举的 Debug 字符串） |

**query**：`{ "vendorTaskId": "…" }`

---

#### 槽 `avatar`

**submit**

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `imagePath` | `string` | 驱动用图像路径（宿主本地路径语义） |
| `audioPath` | `string?` | 驱动用音频路径 |
| `prompt` | `string?` | 附加提示 |
| `extra` | `object?` | 附加上下文 |

**query**：`{ "vendorTaskId": "…" }`

---

#### 槽 `lipsync`

**submit**

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `audioPath` | `string` | 输入音频路径 |
| `inputMode` | `string?` | 输入模式（枚举 Debug 字符串） |
| `extra` | `object?` | 附加上下文 |

**query** / **cancel**：`{ "vendorTaskId": "…" }`

---

#### 槽 `sfx`

**submit**

| Wire 字段 | 类型 | 含义 |
|-----------|------|------|
| `prompt` | `string` | 音效描述 |
| `aiTaskId` | `string?` | 宿主 AI 任务 id |

---

#### 槽 `voiceDesign` / `videoUpscale` / `videoFaceRefine` / `imageUpscale`

契约闭集已保留。仅当**同版本客户端**存在对应 bridge provider 时再实现；否则不要宣称可生成。

---

## 4. 插件回包（Guest → Host）

信封：`protocolVersion` + `operation` + **恰好一个** `dataJson` 或 `error`。

### 4.1 `error`

| Wire 字段 | 含义 |
|-----------|------|
| `code` | 必须是契约 `errorCodes` 之一 |
| `message` | 日志 / 诊断说明；用户文案由宿主按 `code` 映射 |
| `retryable` | 是否可重试 |
| `vendorCode` | 可选厂商错误码 |
| `vendorRequestId` | 可选厂商请求 id |

### 4.2 `submit` 成功（`dataJson` 对象）

由 SDK `SubmitResult` 序列化（`tag = kind`）。

#### `kind: "accepted"`（异步）

| 字段 | 含义 |
|------|------|
| `vendorTaskId` | **真实**厂商任务号；宿主凭此轮询与恢复；禁止编造 |
| `appliedParams` | 可选；见 4.5 |

#### `kind: "completed"`（同步完成）

| 字段 | 含义 |
|------|------|
| `outputs` | 最终成品 `Output[]` |
| `appliedParams` | 可选；见 4.5 |

### 4.3 `query` 成功

| 字段 | 含义 |
|------|------|
| `status` | 契约任务状态之一：`queued` / `running` / `succeeded` / `failed` / `cancelled` / `expired` |
| `outputs` | 完成时的成品（进行中可空） |
| `progressText` | 可选进度文案 |

部分宿主路径也会读数值 `progress`；优先实现契约字段。

### 4.4 `Output`

| 字段 | 含义 |
|------|------|
| `mediaKind` | 如 `image` / `video` / `audio` |
| `source` | `url` \| `host-media` \| `inline-base64` \| `inline-binary` |
| `value` | URL、host media handle 或 Base64 等；**规范键是 `value`**（遗留 `url` / `data` 不当主键） |
| `mime` | 可选 MIME |

文本同步完成时，宿主也可能从 `outputs[0].text`（或兼容键）读正文；新插件应对齐版本化宿主期望与本文。

### 4.5 `appliedParams`（可调参数自证）

当 RCD 对 `duration` / `fps` / `resolution` / `aspect` 声明了带 `bindingId` 的 `range` / `enum` 时，submit 成功响应应带上实际写入厂商请求的值：

| 字段 | 含义 |
|------|------|
| `duration` | 已应用时长 |
| `fps` | 已应用帧率 |
| `resolution` | 已应用分辨率 |
| `aspect` | 已应用画幅 |

声明了控件却未应用 → **宿主硬失败**。  
SDK：`submit_accepted_with_applied` / `submit_completed_with_applied` + `AppliedParams`。

---

## 5. `capabilities` 回包（运行时能力）

`dataJson` 必须符合 `schemas/runtime-caps.schema.json`：

| 范围 | 要求 |
|------|------|
| 顶层 | `schemaVersion`（1）、`pluginId`、`pluginVersion`、`slots[]` |
| 每个 slot | 必有 `slot`、`modelId`；可选 `duration` / `fps` / `resolution` / `aspect` / `maxReferenceImages` / `supportsFirstLastFrame` / `implementedModeIds` / `featureModules` |
| 参数三态 | `fixed` \| `range` \| `enum`；`range` / `enum` **必须**非空 `bindingId` |
| `enum` option | 必有 `id` / `label` / `value`；可选 `aspectRatio`（闭集五画幅）。图像 `resolution` enum **必须**带 `aspectRatio` |

宿主只认这份文档，不认厂商节点图。

---

## 6. 开发者怎么用

1. 动手前顺序：**本文 → WIT → schema → `sdk/rust` 类型**。
2. 用 Cursor / Codex：把 **本文 + `wit/` + `sdk/rust` + 厂商官方 API/SDK** 一并交给智能体；标本仅可选参考。
3. 联调：设置 → 第三方适配器 → 导入 crate 根 → 在对应工作台选 `adapter:{pluginId}:{model}`。失败时对照本文检查 `payloadJson` / 回包。
4. 客户端升级后，以**同版本**本文与 `bridge/*_provider.rs` 为准；发版说明应点名本文变更。

---

## 7. 非目标

1. 不保证任意 HTTP 厂牌、任意槽位不改宿主就能用。
2. `extra`、本地路径、巨型 Base64 可能演进；稳定合同是信封 + 上表具名字段 + RCD + outputs。
3. L2 hello / probe 变绿 ≠ 可生成。

---

## 8. 变更控制

1. 新增可选字段应由旧 guest 忽略。
2. 既有 wire 字段改名或改语义需要协议或文档化主版本升级，并同时更新宿主与 SDK。
3. 改 `bridge/*_provider.rs` 或 SDK 回包辅助时，必须同步更新中文版与英文版。
