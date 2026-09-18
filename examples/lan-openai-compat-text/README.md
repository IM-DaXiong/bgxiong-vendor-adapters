# AI_AUTHOR_GUIDE · LAN OpenAI-compatible text

This crate is a **guest working example**, not a built-in client vendor. It
POSTs OpenAI-shaped JSON to a **user-configured** HTTP origin. The desktop
client has no Ollama / lan-llama branch.

## Two credential archives (same wasm)

1. Third-party OpenAI-compatible engine: set adapter credential `baseUrl`
   (and Bearer if the engine needs it).
2. This product LAN gateway: `http://<peer-host>:18380` + LAN Bearer.
   Do **not** use llama ports 18080–18089.

`EXAMPLE_BASE_URL` (`http://192.168.0.10:11434`) is a **specimen** only.
Submit without `endpointBaseUrl` fails; it never falls back to that IP.

## HTTP

- Empty probe: `ok` + `live:false`, no `host-http`
- Live probe: `GET {origin}/v1/models`
- Submit: `POST {origin}/v1/chat/completions` (sync `kind:completed`)
- Query is not implemented

`thinking:true` is a hard error unless you change the guest.

## Import

Copy the **crate root**. Set credential `baseUrl`. Pick
`adapter:local.example.lan-openai-compat-text:example-lan-llm` in the **text**
picker.

CI must **not** write "LAN LLM is connected".

## After you edit

```text
cargo test --manifest-path packages/vendor-adapter-contract/v1/examples/lan-openai-compat-text/Cargo.toml
cd ui && npm run check:affected
```
