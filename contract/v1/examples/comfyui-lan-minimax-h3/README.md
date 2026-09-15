# AI_AUTHOR_GUIDE · LAN ComfyUI MiniMax H3 (t2v + turbo i2v/r2v)

This crate is a **guest working example**, not a built-in client vendor. Comfy
HTTP, node bindings, and pinned API prompts live only here. The desktop
client has no MiniMax / LAN Comfy branch.

The job runs on the **user-configured Comfy HTTP origin**. This crate does not
start local ComfyUI and does not call RunningHub.

## Models

| modelId | Pin | Media |
|---|---|---|
| `h3.t2v` | `workflows/h3.t2v.api.json` | none; lightning `139` stays false |
| `h3.i2v.turbo` | `workflows/h3.i2v.turbo.api.json` | required first frame; `139` true |
| `h3.r2v.turbo` | `workflows/h3.r2v.turbo.api.json` | required `referenceImagesB64` (1..=3) |

Unknown `model` is an error. Do not coerce r2v start frames into I2V.

## Binding table

See `docs/HOST_MAPPING.zh-CN.md`. `WORKFLOW_PIN*` are SHA-256 of the API files.
To change a graph: export Comfy **API format**, replace the pin, update every
binding. **Do not guess node ids.**

## HTTP

- Probe: `GET {base}/system_stats` (only when `endpointBaseUrl` is set)
- Upload: `POST {base}/upload/image` (multipart field `image`; magic-byte MIME)
- Submit: `POST {base}/prompt` body `{ prompt, client_id }`
- Query: `GET {base}/history/{prompt_id}`
- Result: `{base}/view?filename=...` (`source=url`)

`vendorTaskId` is Comfy `prompt_id`. Cancel is not implemented
(`adapterCapabilityDenied`).

## Import

Copy the **crate root** (`Cargo.toml` + `manifest.json` + `wit/` + `vendor-sdk/`
+ `src/` + `workflows/`) to a writable disk. Do not select `src/` alone.

Set the adapter credential **baseUrl** to the Comfy origin, for example
`http://127.0.0.1:8188` or your LAN host. `endpointPolicy` is
`userConfigurable` + `allowPrivateNetwork`. Do not commit a live LAN IP.

## Acceptance (honest layers)

1. Install T0: `rustc`, `cargo component`, `wasm32-wasip2`.
2. Settings → Third-party adapters → Import **this folder**.
3. Put the Comfy origin on the adapter credential `baseUrl`.
4. Open the **video workbench**. Pick
   `adapter:local.example.comfyui-lan-h3:h3.t2v` /
   `h3.i2v.turbo` / `h3.r2v.turbo`.
5. Live (author machine only): submit → `/prompt` accept → history success → mp4
   download. CI must **not** write "LAN Comfy is connected".

L2 hello green is **not** "this vendor can generate".

## After you edit

```text
cargo test --manifest-path packages/vendor-adapter-contract/v1/examples/comfyui-lan-minimax-h3/Cargo.toml
cd ui && npm run check:affected
```

On failure, give the agent the latest `logs/frontend-*.log` and
`logs/bgxiong-ai-story-*.log` plus the Settings import compiler pane.
