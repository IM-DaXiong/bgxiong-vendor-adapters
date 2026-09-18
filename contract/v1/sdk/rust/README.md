# Vendor Adapter Rust SDK

> **Field contract (required) / 字段契约（必读）**
> - 中文: [`../HOST_PAYLOAD.zh-CN.md`](../HOST_PAYLOAD.zh-CN.md)
> - English: [`../HOST_PAYLOAD.en.md`](../HOST_PAYLOAD.en.md)
> - Index: [`../HOST_PAYLOAD.md`](../HOST_PAYLOAD.md)
>
> Those documents are the project external standard for host `payloadJson` and guest responses. This README is crate usage only.
> 上述文档是本仓对外标准。本 README 只讲 crate 用法。

Status: **supported**. This is the only language SDK with library code today; Python /
TypeScript / TinyGo are 暂不支持 (see their READMEs).

`bgx-vendor-adapter-sdk` covers the JSON envelope that crosses the host boundary and the
payload builders for `submit` / `query`. The WIT world lives in
[`../../wit/vendor-adapter.wit`](../../wit/vendor-adapter.wit) and is generated in your own
crate with `wit-bindgen`; this SDK stays free of `wit-bindgen` so the same code compiles for
host-side tests and for `bgx-adapter dev`.

Closed sets (`protocolHostMajor`, `operations`, `capabilitySlots`) are read from
crate-local `vendor-adapter-protocol.json` at compile time. This crate does not
read host `app-contracts`, so you can copy `sdk/rust` out of the repo.

## Use

```toml
[dependencies]
bgx-vendor-adapter-sdk = { path = "../bgxiong-ai-story/packages/vendor-adapter-contract/v1/sdk/rust" }
```

```rust
use bgx_vendor_adapter_sdk::{submit_accepted, Invocation, Response};

#[derive(serde::Deserialize)]
struct SubmitPayload { prompt: String }

fn handle(raw: &str) -> Response {
    let inv: Invocation = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => return Response::err("submit", "adapterBadOutput", &e.to_string(), false),
    };
    if let Err(e) = inv.validate() {
        return Response::err(&inv.operation, "adapterCapabilityDenied", &e, false);
    }
    let payload: SubmitPayload = match inv.payload() {
        Ok(p) => p,
        Err(e) => return Response::err("submit", "adapterBadOutput", &e.to_string(), false),
    };
    // Build a request plan and call host-http; the vendor task id comes back from the vendor.
    let _ = payload.prompt;
    submit_accepted("vendor-task-id")
}
```

Rules the host enforces, so build for them from the start:

- Never import `wasi:sockets`, `wasi:filesystem`, or `wasi:cli` environment/process. Only the
  `host-*` interfaces in the WIT world are available; anything else fails at load time with
  `adapterCapabilityDenied`.
- All outbound traffic goes through `host-http`. Credentials are opaque handles, never plaintext.
- `submit` must return `accepted` with a real vendor task id for async vendors. The host resumes
  polling after a restart from that id, so do not invent one.
- `range` / `enum` RCD params require `ParamBinding` (non-empty). Use `RcdParam::range` /
  `RcdParam::enum_of`. There is no constructor that declares adjustable params without a binding
  (INV-VENDOR-ADAPTER-RCD-11).
- After a real vendor write, return `submit_accepted_with_applied` so `appliedParams` matches
  the values the host requested. Declaring a control and dropping it on the vendor request is a
  host-hard failure (INV-VENDOR-ADAPTER-RCD-12). The host never reads your vendor workflow JSON.
- Exactly one of `dataJson` / `error` per response. `Response::validate` catches violations.
- Large binaries stay in `host-media` handles; do not read them into linear memory.

## Build

Rust adapters ship a **precompiled** `adapter.wasm` component, so users need no runtime download:

```bash
cargo component build --release --target wasm32-wasip2
bgx-adapter check .
bgx-adapter pack .
```
