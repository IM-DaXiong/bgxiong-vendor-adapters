# Workflow dumps (author input)

OpenAPI V2 AI App graphs. Node tables in `../src/config.rs` are frozen from these
UI dumps + the official `/openapi/v2/run/ai-app/{id}` apiCallDemo.

| modelId | APP_ID | dump |
|---|---|---|
| `h3.r2v.1slot` | `2108667855574224898` | apiCallDemo only; do not use `ComfyUI-API/.../video_minimax_h3_r2v_turbo_1ref.json` |
| `h3.i2v.turbo` | `2101954317581381633` | `h3.i2v.turbo.ui.json` |
| `h3.r2v.2slot` | `2101956370017906690` | `h3.r2v.2slot.ui.json` |
| `h3.r2v.3slot` | `2101957517390737410` | `h3.r2v.3slot.ui.json` |
| `h3.r2v.4slot` | AI App `2103015554197053441` | `h3.r2v.4slot.ui.json` |
| `h3.fl2va.s20` | `2107978021675294722` | apiCallDemo (no local dump in this crate) |
| `h3.soundtrack.1ref` | `2107982892470198274` | source name `video_minimax_h3_r2v_spectrum_soundtrack_1ref.json`; node ids from apiCallDemo |

`h3.r2v.4slot` submits to `/openapi/v2/run/ai-app/2103015554197053441`. LoadImage nodes are `137`, `139`, `143`, `144`.

Do not copy LAN Comfy H3 pins into cloud tables. Do not guess renumbered nodes.

Weight pin tables in `../src/config.rs` are frozen from these dumps plus the
official `/openapi/v2/run/ai-app/{id}` apiCallDemo. i2v turbo UNet is **fl2va**;
`h3.r2v.1slot` and 2/3/4-slot UNet is **ref2va**. 1-slot does not pin CLIP type/device
and has no adjustable fps node; the v2 package declares native 24fps as a fixed value. Mapped submit always sends those pins; omitting
them depends on App webpage defaults and is not portable.
