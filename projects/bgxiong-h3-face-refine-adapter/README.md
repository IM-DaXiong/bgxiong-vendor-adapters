# H3 Face Refine independent adapter

## A23 saved Comfy service binding

Install `install-1.0.0-a23` with a host supporting the application envelope's `comfyui` binding. It uses the saved Comfy connection and the existing GPU/start scheduler; no fixed test port or separate companion process. Package-owned nodes are hash-verified and mounted by the existing Python Comfy startup. Runtime and model versions remain unchanged. Build the wasm32-wasip2 release, then run `Package-Install.ps1`; distribution overwrite is rejected. Stop the idle service after installation, select A23, then analyze targets. GUI cold-start acceptance remains with the user.

- **Layout choice**: sibling repo `<face-adapter-root>` (not a git subtree). Worktree `wt-face-b` / branch `codex/face-b` holds pointers + A03 notes only.
- **Upstream lock**: ComfyUI-H3-FaceRefine **1.1.2** `d8521d14fe0d721d80cd9417fff5a559cbc21aba`.
- **Slot**: `videoFaceRefine` with ops `subjects.analyze`, `subjects.preview`, `video.refine`.
- **Selection profile**: package-local `schemas/media-target-selection-v1/` until Lane A merges public profile (rebase then).

## Isolation (A03)

No global `custom_nodes` overwrite. Companion mounts package-owned upstream via Comfy `--extra-model-paths-config`. Public `local-runtime-v1` is sufficient — **no host port changes**.

## Build

```powershell
$env:CARGO_TARGET_DIR = '<face-adapter-root>\target'
rustup run 1.95.0 cargo build --release --target wasm32-wasip2 -j 2
```

## Run three ops

See `harness/OPS-CHECKLIST.md`. Live user media stays local via env vars.

## A21 model pairing

The full refine graph pins the author-published Kijai 8-step rank-24 LoRA in `companion/models.json`. Install with `./companion/scripts/install-models.ps1 -ModelsRoot '<ComfyUI>/models'`. The installer verifies pinned revision, size and SHA256 before publishing the file; an existing mismatched file is rejected. The original pruned int8_convrot base and user parameters remain unchanged. This LoRA has 208 matching A/B pairs and no trained AdaLN keys; no weights are truncated or skipped. The locked FaceRefine example uses an earlier v1.0 filename: this is an explicit choice of the author's resized version, not a renamed file. The former v4 has incompatible full-width AdaLN weights. Runtime/upstream versions remain unchanged.
