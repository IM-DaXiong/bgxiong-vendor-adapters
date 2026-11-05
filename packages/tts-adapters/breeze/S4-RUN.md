# S4: stream / batch (T18-T21)

## Pull (this turn)
`2e6af9ab0` -> `72818c7fd` (ff-only; dirty stashed then restored)

## Artifacts
| Pack | Version | adapter.wasm sha256 |
|---|---|---|
| A dist-pack/ | 1.0.0-a | 40570a1ead1b92bc233d1650d9feeeff518eede5b6c75b20fbcc8bcd60074a3f |
| B dist-pack-b/ | 1.0.0-b | 12aea70e2d8f4030cf746c6b7fc8b052e2af67f4d3b7314ff2c3e898b4ea4b6f |
| C dist-pack-c/ | 1.0.0-c | 0362d786bafcc80d3df2c6ee8a74114394373ad0724a774c5a1a67eb27a70a56 |

## Host bridge (T18)
`adapter_host/v2/process.rs` collects SDK `workspace/cli/sha256(requestId)/events.jsonl` and mirrors `stream-events.jsonl`; `run_with_host` returns events.

## Product entries
- `speech/tts_streaming.rs` / `installed_speech::synthesize_stream`
- `speech/tts_batch.rs` / `installed_speech::synthesize_batch`

## Prove
```powershell
powershell -File packages/tts-adapters/breeze/scripts/proof-s4-c.ps1
```
Outputs: `E:/BreezeTTS2-assets/s4-staging/out/s4_c_batch_*.wav`, `s4_c_stream.wav`.

## T21 / H2
Event-bridge + C-pack optional stream/batch are H2 candidates. Do not claim H2 freeze/release until explicit T21 sign-off. H1 / A/B unchanged.
