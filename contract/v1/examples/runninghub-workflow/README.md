# Isolated RunningHub AI App video specimen

This crate is a **guest specimen**, not a built-in client vendor. AI App URL, API Key,
`webappId`, and node mapping live only here. The desktop client has no RunningHub branch.

The job runs on **RunningHub cloud**. This crate does not start local ComfyUI.

## Release / NSIS ships a reference copy (placeholder Key)

The installer does **not** copy the whole `packages/` tree. `build-before` syncs a slice into
`user-examples/vendor-adapter-v1/` (this crate + WIT + rust SDK + rust template) so users can
copy **this crate folder** (must include `wit/` and `vendor-sdk/`, not only `src/`) to a writable
disk and import that root. `API_KEY` in the shipped copy must stay the placeholder.
A live Key belongs only in the user's own copy, never in Program Files.

This folder is also the git SSOT for authors.

## Risk: Key inside wasm is extractable

`embeddedHeaders: ["authorization"]` lets an unsigned `local.` plugin send `Authorization`.
The Key is compiled into `adapter.wasm`. Anyone with the package can extract it.

Use a **dedicated, low-balance, revocable** Key. Never share the packed plugin. Never commit
a real 32-character Key. The placeholder `REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY` is hard-refused.

## Region

Edit `src/config.rs`:

- `REGION_BASE_URL` is either `https://www.runninghub.ai` or `https://www.runninghub.cn`
- Submit and query use that same host
- The two regions are **not** fallbacks of each other

## Scope

- AI App `webappId` `2084935567606894593` on `.cn` (`apiType=5`) returns **mp4 video**.
- Submit: `POST /task/openapi/ai-app/run` with `webappId` + `apiKey` + `nodeInfoList`.
- Query: `POST /task/openapi/outputs` with `apiKey` + `taskId` (`code` 0 / 804 / 813 / 805).
- Manifest declares the **video** slot only.
- Default `SUBMIT_MODE=mapped_run`: the video workbench prompt is written to `PROMPT_NODE_ID`.
- `capabilities` returns a vendor-neutral RCD derived from filled bindings in `config.rs`.
  Empty `DURATION_NODE_ID` / `FPS_NODE_ID` / `RESOLUTION_NODE_ID` omit those params; the
  workbench hides them. The host never reads this vendor's graph.
  Freeze widget ids from a one-time `GET /api/webapp/apiCallDemo` on your machine. Do not guess.
- Vendor query can stay running for minutes or many hours. The host poll wall is a fuse
  (`DEFAULT_POLL_WALL_TIMEOUT_SECS`, 8 hours), not a promised job length.
- Result URLs last 24 hours: the host must download at terminal success.

Out of scope: V2 `/openapi/v2/run/workflow`, Model API `imageUrls`, webhook, cancel,
guessing node ids, Kling-style camera feature modules, local ComfyUI.

The current V2 MiniMax H3 **1/2/3-slot working plugin** lives in
`../runninghub-minimax-h3-r2v/`. Import that sibling when the model picker must
show the three RunningHub V2 workflows. The old two-slot crate
`runninghub-minimax-h3` is retired. Do **not** copy V2 bindings into this
AI App teaching specimen.

## End-to-end (video workbench only)

1. Copy this crate folder to a writable disk. Put a dedicated 32-character Key in `API_KEY`.
   Do not commit it.
2. Install once on this machine: `rustc`, `cargo component`, target `wasm32-wasip2`.
   These are **not** inside the desktop installer.
3. Settings → Third-party adapters → Import package → select **this folder**.
4. Open **生成视频** (video generation workbench). Pick model
   `adapter:local.example.specimen:workflow` (display name follows the plugin).
5. Enter a prompt. Duration / fps / resolution stay hidden until you fill the matching
   `*_NODE_ID` in `config.rs`. That is expected.
6. Start generation. Confirm the task is accepted, query reaches `code=0` with an mp4
   `fileUrl`, and the host downloads the 24h URL to disk.
7. Restart while running and confirm a terminal host version.
8. Restore the placeholder Key before any commit.

Do **not** use the image workbench for this specimen. The slot is video / mp4.

L2 hello green is **not** "this vendor can generate".

Optional CLI (same compile SSOT):

```text
cargo test
cargo run --bin bgx-adapter -- build packages/vendor-adapter-contract/v1/examples/runninghub-workflow
```

Without a real Key, only mapper unit tests and anonymous Gateway tests are valid.
Do not write "RunningHub is connected" from CI.
