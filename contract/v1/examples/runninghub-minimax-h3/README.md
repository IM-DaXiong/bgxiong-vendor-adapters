# AI_AUTHOR_GUIDE · RunningHub MiniMax H3 (OpenAPI V2)

This crate is a **guest working example**, not a built-in client vendor. Vendor
URL, API Key, `WORKFLOW_ID`, and node bindings live only here. The desktop
client has no RunningHub branch.

The job runs on **RunningHub cloud**. This crate does not start local ComfyUI.

## Why this crate is V2 (do not merge with the sibling)

| | This crate (`runninghub-minimax-h3`) | Sibling (`runninghub-workflow`) |
|---|---|---|
| Role | Full H3 binding working example | AI App empty-binding teaching specimen |
| pluginId | `local.example.runninghub-h3` | `local.example.specimen` |
| HTTP | `/openapi/v2/run/workflow/{id}` + `/openapi/v2/query` + `/media/upload/binary` | `/task/openapi/ai-app/run` + `/outputs` |
| Auth | Bearer | body `apiKey` |
| Query envelope | `status` / `results` only | `code` only |
| Timing / frames | Bound (see table) | Empty; workbench hides params |

Calling AI App `run` with this `WORKFLOW_ID` returns **`webapp not exists`**.
Do not add a second parse path.

## Binding table (official export of this graph only)

| Host / RCD | Node / field | Notes |
|---|---|---|
| prompt | `134` / `prompt` | Workbench text |
| durationSeconds | `205` / `select` | ImpactSwitch floor 5s: user `1..4` → 5s / `select=1`; `5..15` → `select=seconds-4`. `appliedParams.duration` stays the **user** value. |
| fps | `130` / `fps` **and** `132` / `expression` | Closed set 24/25/30; rewrite both atomically |
| resolution | `115` / `aspect_ratio` | RCD id `16:9` → combo `16:9 (Widescreen)` |
| start frame | `139` / `image` | After upload; magic-byte filename/MIME |
| end frame | `206` / `image` | Same |

`WORKFLOW_ID` `2084935567606894593` is a case-source id, not a secret. Another
account is **not** promised to run this graph after only swapping the Key.
To change the graph: clone/export the vendor graph, then update the id **and**
every binding from that dump. **Do not guess node ids.**

## Images

Decode Base64 → sniff png/jpeg/webp/gif → upload with matching filename and
MIME. Unknown magic **stops**. Do not default to PNG. Do not drop frames and
become T2V.

## Release / NSIS ships a reference copy (placeholder Key)

`build-before` syncs this folder into `user-examples/vendor-adapter-v1/examples/`.
Copy the **crate root** (`Cargo.toml` + `manifest.json` + `wit/` + `vendor-sdk/`
+ `src/`) to a writable disk. Do not select `src/` alone.

## Risk: Key inside wasm is extractable

Use a dedicated, low-balance, revocable Key in the **private copy** only.
Never commit a real 32-character Key. Placeholder
`REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY` is hard-refused. Never ship wasm
that contains a live Key.

## Region

Edit `src/config.rs`: `REGION_BASE_URL` is either `https://www.runninghub.ai`
or `https://www.runninghub.cn`. The two regions are **not** fallbacks.

## Acceptance (honest layers)

1. Copy crate → put Key in `API_KEY` (private copy).
2. Install T0 on this machine: `rustc`, `cargo component`, `wasm32-wasip2`.
3. Settings → Third-party adapters → Import **this folder**.
4. Open the **video workbench** (**生成视频**). Pick `adapter:local.example.runninghub-h3:workflow`.
5. Duration / fps / resolution and up to two reference frames are visible.
6. Live (author machine only): submit → V2 accept → query `SUCCESS` + mp4
   download → restart reclaim. Placeholder Key must block the network.
7. Restore the placeholder before any commit.

L2 hello green is **not** "this vendor can generate".
CI must **not** write "RunningHub is connected".

## After you edit

```text
cargo test --manifest-path packages/vendor-adapter-contract/v1/examples/runninghub-minimax-h3/Cargo.toml
cd ui && npm run check:affected
```

On failure, give the agent the latest `logs/frontend-*.log` and
`logs/bgxiong-ai-story-*.log` plus the Settings import compiler pane.
