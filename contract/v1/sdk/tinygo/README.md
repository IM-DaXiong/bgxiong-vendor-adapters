# Vendor Adapter TinyGo SDK

状态：**暂不支持**（no library code yet）

TinyGo needs no runtime download — like Rust it produces a precompiled `adapter.wasm`
component — but this SDK ships no Go bindings or template, so there is nothing here to build
against. Use the Rust SDK ([`../rust`](../rust)), which is the supported path, or a
`profile.json` adapter for plain HTTP vendors.

If you write the bindings yourself, the target is the same WIT world
([`../../wit/vendor-adapter.wit`](../../wit/vendor-adapter.wit)) and the same closed sets in
`packages/app-contracts/v1/vendor-adapter.json`. The host rules in the Rust README apply
unchanged: no `wasi:sockets` / `wasi:filesystem` / process imports, all traffic through
`host-http`, opaque credential handles, exactly one of `dataJson` / `error` per response.
Adjustable duration/fps/resolution must be declared with a non-empty binding and echoed in
`appliedParams` on submit; the host does not parse vendor workflow JSON.
