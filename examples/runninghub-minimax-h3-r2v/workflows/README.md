# Workflow dumps (author input)

OpenAPI V2 AI App graphs. Node tables in `src/config.rs` are frozen from these
UI dumps + the official `/openapi/v2/run/ai-app/{id}` apiCallDemo.

| modelId | APP_ID | dump |
|---|---|---|
| `h3.r2v.1slot` | `2101954317581381633` | `h3.r2v.1slot.ui.json` |
| `h3.r2v.2slot` | `2101956370017906690` | `h3.r2v.2slot.ui.json` |
| `h3.r2v.3slot` | `2101957517390737410` | `h3.r2v.3slot.ui.json` |
| `h3.r2v.4slot` | AI App `2103015554197053441` | `h3.r2v.4slot.ui.json` |

`h3.r2v.4slot` submits to `/openapi/v2/run/ai-app/2103015554197053441`. LoadImage nodes are `137`, `139`, `143`, `144`.

Do not copy LAN Comfy H3 pins into cloud tables. Do not guess renumbered nodes.
