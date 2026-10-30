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
