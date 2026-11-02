# Upgrade notes: vendor adapter protocol / WIT 1.1.0

WIT package is now `bgxiong:vendor-adapter@1.1.0`. This is an ABI change. Rebuild every WASM guest. Do not keep `1.0.0` and change the binary layout.

## Must recompile

- `wit/vendor-adapter.wit` and crate-local `vendor-sdk/`
- Guest `wit_bindgen::generate!` against 1.1.0
- Host `src-tauri` bindgen (already 1.1.0)

## Invocation envelope

| Old | New |
|-----|-----|
| `deadlineMs` / WIT `deadline-ms` as wall-clock | `budgetMs` / `budget-ms` remaining execution budget |
| negotiated fields JSON-only | WIT `binding-id`, `negotiated-version`, `enabled-features` |
| host `aiTaskId` on the guest envelope | host-private; guest sees `requestId` + `bindingId` |

## Output

Submit/query `outputs[]` items are a tagged union: `kind: "text"` or `kind: "media"`. Do not send a bare string as the only success body.

## Cancel

If the plugin cannot cancel a vendor job, return `adapterCapabilityDenied` / documented unsupported. Do not report cancelled.

## Profile runner

In-host profile is a **dev-tool subset**. Compile to a standard guest for production plugins.

## In-flight tasks

Old 1.0.0 components will not instantiate on a 1.1.0 host. Finish or explicitly cancel running 1.0 tasks before switching the host binary. Do not silently route new work to stale artifacts.

## Host HTTP `timeout-ms` (INV-ADAPTER-HOST-WAIT)

| Value | Meaning |
|-------|---------|
| `0` (default for submit/query/cancel/upload/download) | Do **not** set a reqwest whole-request timeout. Guest CPU `budgetMs`/`wallMs` is unrelated. The worker process still has an independent safety fuse from `network-resilience.json`. |
| Positive | Author-chosen short request cap. Host rejects values above `adapterHostRequestTimeoutMaxMs`. |

Do not copy guest `wallMs` (15s rust tier) into HTTP. Do not copy the 8h poll wall into the worker.
