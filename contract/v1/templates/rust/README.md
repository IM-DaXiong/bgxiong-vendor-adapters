# local.example.starter

Anonymous Rust adapter template. `bgx-adapter init rust` copies this crate.

Unsigned rust crate: Settings → Third-party adapters → Import package → select this
folder. The client compiles with `cargo component build --release --target wasm32-wasip2`.
You need `rustc`, `cargo-component`, and `wasm32-wasip2` on this machine (not in the
desktop installer).

Optional CLI (same SSOT):

```text
cargo run --bin bgx-adapter -- build .
```

Import L3 `probe` is offline (`{"ok":true}`). Do not call `host-http` until the
host injects `endpointBaseUrl`. Replace submit/query URL, body, and mapping with
your vendor. Do not add vendor names to the client; keep them in this crate.

WIT world: crate-local `wit/vendor-adapter.wit` (`bgxiong:vendor-adapter@1.1.0`).
SDK envelope: crate-local `vendor-sdk/` (embeds `vendor-adapter-protocol.json`).
Do not depend on host `app-contracts` or `src-tauri`.

L2 hello green is not generate-ready.
