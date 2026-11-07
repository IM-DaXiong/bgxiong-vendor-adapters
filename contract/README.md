# vendor-adapter-contract

External developer contract for BigBear / bgxiong-ai-story third-party vendor adapters.

对外开发契约：比格熊数字导演 · 第三方算力适配器。

## Start here / 从这里读

1. **Field dictionary / 字段辞典**
   - 中文: [v1/HOST_PAYLOAD.zh-CN.md](v1/HOST_PAYLOAD.zh-CN.md)
   - English: [v1/HOST_PAYLOAD.en.md](v1/HOST_PAYLOAD.en.md)
   - Index: [v1/HOST_PAYLOAD.md](v1/HOST_PAYLOAD.md)
2. WIT: [v1/wit/vendor-adapter.wit](v1/wit/vendor-adapter.wit)
3. Rust SDK: [v1/sdk/rust](v1/sdk/rust)
4. Schemas: [v1/schemas](v1/schemas)
5. Other language SDKs, all **暂不支持** / not supported (no library to build):
   - [Python](v1/sdk/python/README.md)
   - [TypeScript](v1/sdk/typescript/README.md)
   - [TinyGo](v1/sdk/tinygo/README.md)
6. Rust template: [v1/templates/rust](v1/templates/rust)

`v1/examples/` are specimens only, not the standard.

`v1/examples/` 仅为标本，不是标准。

## SDK V2 / 开发者预览

V2 是开发者预览，不是已封存的稳定 SDK。[release-lock.json](v2/release-lock.json) 里 `sdkReady` 为 `false`。从这里读：

1. [v2/README.md](v2/README.md)
2. [预览边界](v2/DEVELOPER-PREVIEW.zh-CN.md)
3. [开放标准](v2/OPEN-STANDARD.zh-CN.md)
4. [SDK 接口](v2/SDK-CONTRACT.zh-CN.md)
5. [作者指南](v2/AUTHORING.zh-CN.md)
6. [符合性条件](v2/CONFORMANCE.zh-CN.md)

不要把预览样例或 `sdkReady=false` 写成已经稳定发布。
