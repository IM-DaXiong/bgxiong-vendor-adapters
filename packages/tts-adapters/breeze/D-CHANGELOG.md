# Breeze D-pack 1.0.0-d (S1 capability pack)

Relative to A (1.0.0-a); A/B/C bytes unchanged under dist-pack / dist-pack-b / dist-pack-c:

- version: `1.0.0-d`
- adapter.wasm sha256: `07b18217f4f5cf03fd18668cc80944cc108534f7f55eecf4e8496d636e053d08`
- tts-bundle.json: real adapter + `runtimes[].code` digests (no PENDING_*)
- `runtimes[].code.path` = `companion/files.json` (inventory file, not a directory)
- `runtimes[].code.sha256` = raw SHA-256 of that inventory file
- companion inventory lists 52 managed code files (excludes tests/__pycache__/.venv); paths relative to `companion/`
- install/provisioning.json points pluginVersion/adapterSha256 at D; runtime 1.0.2 / deps 1.0.1 / model / python / sox entries unchanged
- packaging scripts copy D pack + tts-bundle.json + companion/ into offline `adapter-pack/`

Artifact: `packages/tts-adapters/breeze/dist-pack-d/`
