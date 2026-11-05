# Pointer: H3 Face Refine independent adapter (Lane B)

**Choice (documented):** implement as **sibling** directory, not git subtree and not a full copy under this path.

| Item | Path |
|------|------|
| Sibling package (SSOT for A03–A05) | `L:\00Dev\bgxiong-h3-face-refine-adapter` |
| Sibling tip (local) | `a21bdc1` (wasm + install envelope) |
| Install envelope (human acceptance) | `L:\00Dev\bgxiong-h3-face-refine-adapter\install` |
| Worktree / branch | `L:\00Dev\wt-face-b` / `codex/face-b` |
| Upstream lock | ComfyUI-H3-FaceRefine 1.1.2 `d8521d14fe0d721d80cd9417fff5a559cbc21aba` |
| Package version | `1.0.0-a04` / pluginId `local.thirdparty.h3-face-refine` |

## Why sibling

Plan §3.4 requires an independent author tree that does not relative-reference the main repo. Sibling matches that; `packages/third-party/...` here is only a pointer so the branch records the decision without vendoring upstream node trees into the product monorepo.

## A03

Generic local-runtime isolation via companion `extra_model_paths` — **no global custom_nodes overwrite**. Existing public `local-runtime-v1` sufficient; **no host port extension**.

## A02 dependency

**Aligned:** public profile present from Lane A `3b6a7e738` at `packages/vendor-adapter-contract/v2/profiles/media-target-selection-v1/`. Sibling mirrors the same files under `schemas/media-target-selection-v1/` for independent builds.

## Build / install (acceptance)

```powershell
# sibling build (toolchain 1.95.0)
cd L:\00Dev\bgxiong-h3-face-refine-adapter
powershell -NoProfile -ExecutionPolicy Bypass -File .\Build.ps1 -Jobs 2
# install outer folder into INT app (auth=none):
#   L:\00Dev\bgxiong-h3-face-refine-adapter\install
# managed Comfy prepare (does not start Comfy):
#   companion\scripts\prepare-runtime.ps1 -RuntimeRoot L:\00Dev\runtimes\h3-face-refine-1.1.2
```

See sibling `install/README-ACCEPTANCE.md` and `harness/OPS-CHECKLIST.md`.
