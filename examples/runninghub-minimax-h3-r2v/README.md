# AI_AUTHOR_GUIDE · RunningHub MiniMax H3 r2v 1/2/3 slots (OpenAPI V2 AI App)

This crate is a **guest working example**, not a built-in client vendor. Vendor
URL, API Key, app ids, and node bindings live only here. The desktop client has
no RunningHub branch. E2E live path is the **video workbench** (cover-import
this crate, pick `h3.r2v.1slot` / `2slot` / `3slot`, submit).

## HTTP

| | This crate | Legacy specimen `runninghub-workflow` |
|---|---|---|
| pluginId | `local.example.runninghub-h3-r2v` | `local.example.specimen` |
| Submit | `POST /openapi/v2/run/ai-app/{APP_ID}` | legacy task-envelope AI App runner |
| Query | `/openapi/v2/query` (top-level `taskId`) | `/task/openapi/outputs` (`code` envelope) |
| Upload | `/openapi/v2/media/upload/binary` | different |

## Binding table

| modelId | APP_ID | LoadImage | prompt |
|---|---|---|---|
| `h3.r2v.1slot` | `2101954317581381633` | `114` | `132/prompt` |
| `h3.r2v.2slot` | `2101956370017906690` | `137`,`139` (never `141`) | `138/value` |
| `h3.r2v.3slot` | `2101957517390737410` | `137`,`139`,`143` (never `141`) | `138/value` |

1-slot duration `134`; fps optional `131`+math `133`. 2/3-slot duration `132`;
fps `130`+math `131`. Aspect/mp `115`. Do **not** share widgets across models.
Do **not** guess node ids. Dump authority: `workflows/h3.r2v.*.ui.json`.

`nodeInfoList[].fieldValue` is always a **string**. Only workbench fields are
pushed (no VAE/UNET/CLIP replay).

## Images

Host stages files (`video-reference-media-v1`). Guest uploads by handle.
All three declare `multi_image_to_video` only. Do not treat 1-slot as I2V in RCD.

## Import

Copy the **crate root**. Cover-import after each version bump. Live requires a
real 32-char Key in a private copy (placeholder is hard-refused).

## Acceptance

L2 hello green is **not** generation. Live: each picker row submits its own
`/run/ai-app/{id}` and returns top-level `taskId`. CI must **not** write
"RunningHub is connected".

```text
cargo test --manifest-path packages/vendor-adapter-contract/v1/examples/runninghub-minimax-h3-r2v/Cargo.toml
cd ui && npm run check:affected
```
