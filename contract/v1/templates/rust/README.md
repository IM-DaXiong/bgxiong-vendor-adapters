# local.example.starter

Anonymous Rust adapter template. Copy this directory to start a new adapter.

Unsigned rust crate: Settings → Third-party adapters → Import package → select this
folder. The client compiles with `cargo component build --release --target wasm32-wasip2`.
You need `rustc`, `cargo`, `cargo-component`, and the `wasm32-wasip2` target on this machine
(not in the desktop installer).

You can also build the component yourself:

```text
cargo component build --release --target wasm32-wasip2
```

Import L3 `probe` is offline (`{"ok":true}`). Do not call `host-http` until the
host injects `endpointBaseUrl`. Replace submit/query URL, body, and mapping with
your vendor. Do not add vendor names to the client; keep them in this crate.

WIT world: crate-local `wit/vendor-adapter.wit` (`bgxiong:vendor-adapter@1.1.0`).
SDK envelope: crate-local `vendor-sdk/` (embeds `vendor-adapter-protocol.json`).
Do not depend on private host source. The SDK and WIT in this folder are enough.

L2 hello green is not generate-ready.
