# SDK 接入文档

这份文档说明如何用**本仓库里的 SDK 和契约**，做出能导入比格熊数字导演工作站的第三方适配器。

配套英文：[`SDK-INTEGRATION.en.md`](SDK-INTEGRATION.en.md)。字段逐项说明仍以 [`../contract/v1/HOST_PAYLOAD.zh-CN.md`](../contract/v1/HOST_PAYLOAD.zh-CN.md) 为准。

## 版本

| 项 | 值 |
|---|---|
| 契约发布 | 1.2.0 |
| WIT | `bgxiong:vendor-adapter@1.1.0`（`contract/v1/wit/vendor-adapter.wit`） |
| Manifest | `schemaVersion: 2` |
| Rust SDK | `contract/v1/sdk/rust`，crate 名 `bgx-vendor-adapter-sdk`，版本 1.2.0 |
| 模板 | `contract/v1/templates/rust` |

1.2.0 要求：query 结果为 `failed`、`cancelled` 或 `expired` 时，必须带非空 `terminalFailure`。WIT 包版本仍是 1.1.0。

## 接入模型

工作站每次调用 guest 导出的 `invoke`，传入 Invocation 信封。业务字段在字符串 `payloadJson` 里。

```text
Host                                        Guest
Invocation {                                Response {
  protocolVersion,                            protocolVersion,
  operation,          // capabilities|probe|upload|submit|query|cancel
  profileId,                                  dataJson 或 error（只能有一个）
  capabilitySlot,     // image|video|text|...
  requestId,                                }
  budgetMs,
  payloadJson,
  bindingId,
  negotiatedVersion?,
  enabledFeatures?
}
```

Guest 只能导入 WIT 世界里的 host 接口：

| 接口 | 用途 |
|---|---|
| `host-http` | 全部出网。凭证是不透明句柄，不是明文密钥 |
| `host-media` | 大文件的读、写、完成 |
| `host-credential` | 询问凭证能力、签名 |
| `host-state` | 小块状态 |
| `host-log` | 日志 |

禁止导入 `wasi:sockets`、`wasi:filesystem`、`wasi:cli`。否则加载阶段会以 `adapterCapabilityDenied` 失败。

操作闭集、能力槽、错误码在 `contract/v1/protocol/vendor-adapter-protocol.json`。Rust SDK 编译时嵌入同内容的 `vendor-adapter-protocol.json`。

## 用 Rust SDK

在新适配器的 `Cargo.toml` 里依赖本仓库的 crate，或依赖模板/示例自带的拷贝：

```toml
[dependencies]
bgx-vendor-adapter-sdk = { path = "vendor-sdk" }
```

若适配器放在本仓库外、且没有拷贝 `vendor-sdk`，把路径指到 `contract/v1/sdk/rust`。

```rust
use bgx_vendor_adapter_sdk::{submit_accepted, Invocation, Response};
```

SDK 负责信封校验、`submit` / `query` 回包辅助类型（`Invocation`、`Response`、`SubmitResult`、`QueryResult`、`Output`）。WIT 绑定由适配器自己用 `wit-bindgen` 生成，SDK 不依赖 `wit-bindgen`，因此同一份代码可以在主机侧 `cargo test`，也可以编进 `wasm32-wasip2`。

从 `contract/v1/templates/rust` 复制时，这些已经接好：

- `wit/vendor-adapter.wit`
- `vendor-sdk/`（内含协议 JSON）
- `manifest.json`
- `src/lib.rs` 里的 `invoke` 分发

模板的导入探测返回 `{"ok":true}`，并且不调用 `host-http`。只有宿主注入了非空 `endpointBaseUrl` 时才做出网探活。把模板里的 submit/query 换成厂商 URL、请求体和字段映射。

实现时遵守：

- 异步厂商的 submit 必须 `accepted`，并带上真实 `vendorTaskId`。工作站重启后靠这个 id 继续轮询，不能编造。
- 声明了可调参数就要写入厂商请求，并用 `submit_accepted_with_applied` 回显 `appliedParams`。
- `range` / `enum` 参数使用 SDK 的 `RcdParam::range` / `RcdParam::enum_of`，binding 不能空。
- 大二进制走 `host-media` 句柄，不要默认读进线性内存。
- 用户能看见的 `error.message` 写人话。错误码用契约里的 `error.code`，不要把堆栈塞进去。

更完整的用法见 [`../contract/v1/sdk/rust/README.md`](../contract/v1/sdk/rust/README.md)。

本发布版的可编译 SDK 是 Rust。`contract/v1/sdk/python`、`typescript`、`tinygo` 说明同一份 WIT 和闭集，目录里还没有第二份库。简单 HTTP 也可以用 manifest `language: "profile"` 配 `schemas/profile.schema.json`，生产插件仍建议编成标准 Rust guest。

## Host 字段怎么映射

Wire 名一律 camelCase。未在辞典列出的键不要依赖；未知键应忽略。缺省或 JSON `null` 表示未提供。

常见槽位（完整表在字段辞典第 3 节）：

| 槽 | submit 里常见字段 |
|---|---|
| `image` / `imageToImage` | `prompt`、`model`、`size`（`WxH`）、`negativePrompt`、`referenceImagesB64` |
| `video` / `keyframe` | `prompt`、`model`、`resolution`、`fps`、`durationSeconds`、首尾帧与参考图 |
| `text` | 以标本 `examples/lan-openai-compat-text` 和辞典为准 |

`model` 是 `adapter:{pluginId}:` **后面**的那段，对应 manifest 里的 `modelId`。

启用可选特性 `video-reference-media-v1` 后，视频 submit 改用 `MediaInputV1` 句柄（`startFrame`、`endFrame`、`referenceImages`、`referenceVideos`、`referenceAudios`），不要和旧的 `*B64` 字段出现在同一次提交里。Schema：`contract/v1/schemas/video-media-input.schema.json`。离线对照：`examples/video-reference-media-probe`。

图像尺寸：submit 的 `size` 是工作台选的 `WxH`；运行时能力里的 `resolution` 选项 `value` 与它相同；回包 `appliedParams.resolution` 必须等于实际写入厂商请求的 `WxH`。

凭证里的 `baseUrl` 由宿主写成顶层 `endpointBaseUrl`（submit、query、以及带地址的 probe）。这是通用字段，不是某一家厂商的 API。

## 打包

一个可导入的 Rust 适配器目录至少包括：

```text
your-adapter/
  manifest.json
  Cargo.toml
  README.md
  src/
  wit/vendor-adapter.wit
  vendor-sdk/
```

`manifest.json` 必填：

- `schemaVersion`: `2`
- `pluginId`、`displayName`、`version`
- `language`: `rust`
- `entry`: `adapter.wasm`
- `capabilitySlots`: 非空数组，每项有 `slot`
- `requestedCapabilities.endpointPolicy`：`staticHosts`、`userConfigurable`、`allowPrivateNetwork` 的组合
- `requestedCapabilities.auth`：例如 `none`、`bearer`

`staticHosts` 时列出 `networkHosts`。局域网地址用 `userConfigurable` 加 `allowPrivateNetwork`。需要在未签名 `local.` 插件里发送 `Authorization` 时，按标本声明 `embeddedHeaders: ["authorization"]`，并记住密钥会进 wasm，只能使用可撤销的专用 Key。

标本 manifest 可以直接抄结构：`examples/comfyui-lan-z-image-turbo/manifest.json`、`examples/runninghub-workflow/manifest.json`。

## 校验

```bash
python3 scripts/validate-example.py examples/comfyui-lan-z-image-turbo
python3 scripts/validate-example.py path/to/your-adapter
```

脚本检查 manifest 必填项、`schemaVersion == 2`，并拒绝明显的私钥块、`sk-` 形态和写死的非占位 `API_KEY`。它不编译 WASM。

有工具链时在适配器目录执行：

```bash
cargo test
cargo component build --release --target wasm32-wasip2
```

## 导入比格熊

1. 安装工作站：<https://www.bgxiong.com>
2. 打开 **设置 → 第三方适配器 → 导入插件包**
3. 选择适配器**目录根**，不要只选 `src/`
4. 本机需已安装 `rustc`、`cargo`、`cargo-component` 和 `wasm32-wasip2`。客户端用 `cargo component build --release --target wasm32-wasip2` 编译
5. 导入后按提示重启客户端
6. 填写凭证（`baseUrl` 和/或 Bearer）。先做探测，再在对应工作台选择 `adapter:{pluginId}:{modelId}`

导入阶段的空探测没有厂商地址，guest 必须直接成功，不能访问网络。探测通过不等于已经能生成。厂商权重、工作流节点和 Key 仍要在目标环境里就绪。

## 密钥

- 仓库和示例只保留占位符：`REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY`
- 真实 Key 只放在使用者自己的副本或客户端凭证里
- 不要把带真实 Key 的 wasm 发给别人。编译进 wasm 的 Key 可以被提取
