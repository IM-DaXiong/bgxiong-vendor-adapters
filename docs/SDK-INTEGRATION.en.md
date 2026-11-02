# SDK integration

Use the SDK, WIT, schemas, and examples **in this repository** to build a third-party adapter that imports into the BGXiong workstation.

Chinese edition: [`SDK接入.md`](SDK接入.md). Field-by-field rules: [`../contract/v1/HOST_PAYLOAD.en.md`](../contract/v1/HOST_PAYLOAD.en.md).

## Version

| Item | Value |
|---|---|
| Contract release | 1.2.0 |
| WIT | `bgxiong:vendor-adapter@1.1.0` (`contract/v1/wit/vendor-adapter.wit`) |
| Manifest | `schemaVersion: 2` |
| Rust SDK | `contract/v1/sdk/rust`, crate `bgx-vendor-adapter-sdk` 1.2.0 |
| Template | `contract/v1/templates/rust` |

Release 1.2.0 requires a non-empty `terminalFailure` on query results whose status is `failed`, `cancelled`, or `expired`. The WIT package version stays 1.1.0.

## Call shape

The workstation calls the guest export `invoke` with an Invocation envelope. Business fields are inside the string `payloadJson`.

Operations: `capabilities`, `probe`, `upload`, `submit`, `query`, `cancel`.

The response carries exactly one of `dataJson` or `error`.

Guest imports are only the WIT host interfaces: `host-http`, `host-media`, `host-credential`, `host-state`, `host-log`. Do not import `wasi:sockets`, `wasi:filesystem`, or `wasi:cli`.

Closed sets live in `contract/v1/protocol/vendor-adapter-protocol.json`. The Rust SDK embeds that document.

## Rust SDK

Adapters in this repo depend on the vendored copy:

```toml
[dependencies]
bgx-vendor-adapter-sdk = { path = "vendor-sdk" }
```

Point `path` at `contract/v1/sdk/rust` when the adapter lives outside this repo and you have not copied `vendor-sdk`.

The crate validates the envelope and builds `submit` / `query` bodies (`Invocation`, `Response`, `SubmitResult`, `QueryResult`, `Output`). Generate WIT bindings in the adapter with `wit-bindgen`. The SDK itself does not depend on `wit-bindgen`.

Copy `contract/v1/templates/rust` to start. Its import probe returns `{"ok":true}` and must not call `host-http`. Call the network only after the host injects a non-empty `endpointBaseUrl`.

Rules:

- Async `submit` returns `accepted` with the vendor's real task id. Do not invent `vendorTaskId`.
- Adjustable parameters must be written on the vendor request and echoed in `appliedParams`.
- Keep large binaries in `host-media` handles.
- Put a contract `error.code` on failures. Keep `error.message` readable.

See [`../contract/v1/sdk/rust/README.md`](../contract/v1/sdk/rust/README.md).

This release ships the Rust library. The `python`, `typescript`, and `tinygo` directories describe the same WIT; they do not contain a second library. A declarative HTTP adapter can use `language: "profile"` and `schemas/profile.schema.json`. Production plugins should still be a Rust guest.

## Mapping host fields

Wire names are camelCase. Ignore unknown keys. Full tables are in the field dictionary, section 3.

| Slot | Typical submit fields |
|---|---|
| `image` / `imageToImage` | `prompt`, `model`, `size` (`WxH`), `negativePrompt`, `referenceImagesB64` |
| `video` / `keyframe` | `prompt`, `model`, `resolution`, `fps`, `durationSeconds`, frames and references |
| `text` | See `examples/lan-openai-compat-text` and the field dictionary |

`model` is the part after `adapter:{pluginId}:`, matching `modelId` in the manifest.

With optional feature `video-reference-media-v1`, video submit uses `MediaInputV1` handles (`startFrame`, `endFrame`, `referenceImages`, `referenceVideos`, `referenceAudios`) and must not also send the legacy `*B64` fields. Schema: `contract/v1/schemas/video-media-input.schema.json`. Offline specimen: `examples/video-reference-media-probe`.

The credential `baseUrl` arrives as top-level `endpointBaseUrl`.

## Package layout

```text
your-adapter/
  manifest.json
  Cargo.toml
  README.md
  src/
  wit/vendor-adapter.wit
  vendor-sdk/
```

Required manifest fields: `schemaVersion` 2, `pluginId`, `displayName`, `version`, `language`, `entry` (`adapter.wasm`), non-empty `capabilitySlots`, and `requestedCapabilities` with `endpointPolicy` plus `auth`.

Use `staticHosts` plus `networkHosts` for fixed vendor hosts. Use `userConfigurable` and `allowPrivateNetwork` for a LAN origin. Copy a specimen manifest rather than inventing keys.

## Validate

```bash
python3 scripts/validate-example.py examples/comfyui-lan-z-image-turbo
python3 scripts/validate-example.py path/to/your-adapter
```

With a Rust toolchain, in the adapter directory:

```bash
cargo test
cargo component build --release --target wasm32-wasip2
```

## Import into BGXiong

1. Install the workstation: <https://www.bgxiong.com>
2. Settings → Third-party adapters → Import package.
3. Select the adapter root, not `src/` alone.
4. The machine needs `rustc`, `cargo`, `cargo-component`, and target `wasm32-wasip2`. The client runs `cargo component build --release --target wasm32-wasip2`.
5. Restart the client if it asks you to.
6. Set `baseUrl` and/or a bearer credential, probe, then pick `adapter:{pluginId}:{modelId}` in the matching workbench.

The import probe has no vendor URL. The guest must succeed without `host-http`. A green probe does not mean generation is ready.

## Secrets

Keep `REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY` in the repository. Real keys belong in the user's credential settings. A key compiled into `adapter.wasm` can be extracted.
