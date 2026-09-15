# Vendor Adapter Python SDK

状态：**暂不支持**（no library code, and no runtime to run it on）

Python adapters need a `cpython.wasm` engine on the client. That artifact has no CDN source
yet, so `runtime_key_for_language("python")` can never reach L1 on a user machine and the
adapter management page shows「暂不提供下载」instead of a probe button. Writing a Python
adapter today produces a plugin that cannot run for any user.

Use the Rust SDK ([`../rust`](../rust)) or a `profile.json` adapter instead. Profile adapters
need no language runtime at all and cover plain HTTP vendors. Adjustable params still follow
Rust README: `bindingId` + `appliedParams`; the host does not read vendor workflow JSON.

Planned entry contract, for reference only: `def invoke(op: str, payload: dict) -> dict`,
no pip and no C extensions. Conformance fixtures: [`../../fixtures/`](../../fixtures/).
