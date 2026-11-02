# Vendor Adapter TypeScript SDK

状态：**暂不支持**（no library code, and no runtime to run it on）

TypeScript adapters need a `js-engine.wasm` engine on the client. That artifact has no CDN
source yet, so `runtime_key_for_language("typescript")` can never reach L1 on a user machine
and the adapter management page shows「暂不提供下载」instead of a probe button. Writing a
TypeScript adapter today produces a plugin that cannot run for any user.

Use the Rust SDK ([`../rust`](../rust)) or a `profile.json` adapter instead. Adjustable params
must declare a binding and echo `appliedParams`; the host does not parse vendor workflow JSON.

Planned entry contract, for reference only:
`export default async function invoke(op: string, payload: unknown): Promise<unknown>`.
Conformance fixtures: [`../../fixtures/`](../../fixtures/).
