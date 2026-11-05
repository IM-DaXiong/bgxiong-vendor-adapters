# Breeze C-pack 1.0.0-c (S4)

Relative to B (1.0.0-b); A/B bytes unchanged:

- optionalFeatures: stream-events-v1
- std/speech.synthesize execution.streaming=true; execution.delivery=stream via host-http-stream (PCM to WAV wrap) + host-stream.emit-event (progress/media_chunk)
- std/speech.batch (per-item complete WAV)

Artifact dist-pack-c adapter.wasm sha256:
`0362d786bafcc80d3df2c6ee8a74114394373ad0724a774c5a1a67eb27a70a56`

Prove: `powershell -File packages/tts-adapters/breeze/scripts/proof-s4-c.ps1`
- S4_C_BATCH_PASS (2 wav)
- S4_C_STREAM_PASS (events.jsonl progress+media_chunk, wav out)
