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

`v1/examples/` are specimens only, not the standard.

`v1/examples/` 仅为标本，不是标准。

Host `payloadJson` never contains vendor `nodeInfoList`. Guest maps HOST_PAYLOAD
to the vendor HTTP body. For overlay-style APIs (App defaults + field list),
authors must pin weight widgets on every submit. See
[GUIDE-VendorAdapter-开发者手册.md](../../docs/dev/guides/GUIDE-VendorAdapter-开发者手册.md)
section 「提交映射：整图 pin vs 增量覆盖」.


## v2 developer preview / v2 开发者预览

Contract release **2.0.0** (author SDK / WIT identity). Installed materials ship as `user-examples/vendor-adapter-v2/` and `user-examples/comfy-workflow-authoring-v2/`.

1. Status & limits: [v2/DEVELOPER-PREVIEW.zh-CN.md](v2/DEVELOPER-PREVIEW.zh-CN.md) — **sdkReady=false**
2. Authoring: [v2/AUTHORING.zh-CN.md](v2/AUTHORING.zh-CN.md) (if present) / [v2/README.md](v2/README.md)
3. **SDK release 2.1.1 panorama candidate**: [v2/profiles/image-projection-v1/README.zh-CN.md](v2/profiles/image-projection-v1/README.zh-CN.md) · [README.en.md](v2/profiles/image-projection-v1/README.en.md)
4. Video generation profile: [v2/profiles/video-generation-v1/README.zh-CN.md](v2/profiles/video-generation-v1/README.zh-CN.md)

`RELEASE-STATUS.json` top-level fields describe standalone full-candidate governance; do not treat them as a denial of the installed workflow-adapter preview.

RunningHub cloud adapters are **independent** packages under `packages/cloud-adapters/`, not v1 examples.

中文：v2 是 ComfyUI 工作流作者开发包·开发者预览；2.1.1 为 image-projection 候选叠加，不是全 SDK 密封发行。已有 v1 厂商插件继续按 v1。
