# H3 Face Refine independent adapter

- **Layout choice**: sibling repo `L:\00Dev\bgxiong-h3-face-refine-adapter` (not a git subtree). Worktree `wt-face-b` / branch `codex/face-b` holds pointers + A03 notes only.
- **Upstream lock**: ComfyUI-H3-FaceRefine **1.1.2** `d8521d14fe0d721d80cd9417fff5a559cbc21aba`.
- **Slot**: `videoFaceRefine` with ops `subjects.analyze`, `subjects.preview`, `video.refine`.
- **Selection profile**: package-local `schemas/media-target-selection-v1/` until Lane A merges public profile (rebase then).

## Isolation (A03)

No global `custom_nodes` overwrite. Companion mounts package-owned upstream via Comfy `--extra-model-paths-config`. Public `local-runtime-v1` is sufficient — **no host port changes**.

## Build

```powershell
$env:CARGO_TARGET_DIR = 'L:\00Dev\bgxiong-h3-face-refine-adapter\target-face-b'
rustup run 1.95.0 cargo build --release --target wasm32-wasip2 -j 2
```

## Run three ops

See `harness/OPS-CHECKLIST.md`. Live user media stays local via env vars.

## A21 model pairing

The full refine graph pins the author-published Kijai 8-step rank-24 LoRA in `companion/models.json`. Install with `./companion/scripts/install-models.ps1 -ModelsRoot '<ComfyUI>/models'`. The installer verifies pinned revision, size and SHA256 before publishing the file; an existing mismatched file is rejected. The original pruned int8_convrot base and user parameters remain unchanged. This LoRA has 208 matching A/B pairs and no trained AdaLN keys; no weights are truncated or skipped. The locked FaceRefine example uses an earlier v1.0 filename: this is an explicit choice of the author's resized version, not a renamed file. The former v4 has incompatible full-width AdaLN weights. Runtime/upstream versions remain unchanged.
