# Vendor Adapter Rust SDK

Status: **supported**. This is the only language SDK with library code today; Python /
TypeScript / TinyGo are 暂不支持 (see their READMEs).

`bgx-vendor-adapter-sdk` covers the JSON envelope that crosses the host boundary and the
payload builders for `submit` / `query`. The WIT world lives in
[vendor-adapter.wit](../../../contract/v1/wit/vendor-adapter.wit) and is generated in your own
crate with `wit-bindgen`; this SDK stays free of `wit-bindgen` so the same code compiles for
host-side tests and for `bgx-adapter dev`.

Closed sets (`protocolHostMajor`, `operations`, `capabilitySlots`) are read from
crate-local `src/vendor-adapter-protocol.json` at compile time. This copy does not read host
app-contracts (that file is not in this repository).

## Use

```toml
[dependencies]
bgx-vendor-adapter-sdk = { path = "contract/v1/sdk/rust" }
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
- Exactly one of `dataJson` / `error` per response. `Response::validate` catches violations.
- Large binaries stay in `host-media` handles; do not read them into linear memory.

## Build

Rust adapters ship a **precompiled** `adapter.wasm` component, so users need no runtime download:

```bash
cargo component build --release --target wasm32-wasip2
bgx-adapter check .
bgx-adapter pack .
```
