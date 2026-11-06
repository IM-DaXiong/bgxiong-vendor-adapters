# Pointer: H3 Face Refine independent adapter

**Layout:** independent package under `projects/bgxiong-h3-face-refine-adapter` (not a git subtree under this path). This file is only a pointer so `packages/third-party/` records the decision without vendoring upstream node trees into product monorepos.

| Item | Path |
|------|------|
| Package SSOT | `projects/bgxiong-h3-face-refine-adapter` |
| Upstream lock | ComfyUI-H3-FaceRefine 1.1.2 `d8521d14fe0d721d80cd9417fff5a559cbc21aba` |
| Package version | see `projects/bgxiong-h3-face-refine-adapter/SOURCE.json` / `manifest.json` |
| pluginId | `local.thirdparty.h3-face-refine` |

## Why independent project

Author trees must not relative-reference the product monorepo for source ownership. This `packages/third-party/` entry stays a pointer; build and install envelopes live with the project.

## A03

Generic local-runtime isolation via companion `extra_model_paths` — **no global custom_nodes overwrite**. Existing public `local-runtime-v1` is sufficient; **no host port extension**.

## A02 dependency

The package mirror for independent builds is [media-target-selection-v1](../../../projects/bgxiong-h3-face-refine-adapter/schemas/media-target-selection-v1/). The public contract in this repository is [media-target-selection-v1](../../../contract/v2/profiles/media-target-selection-v1/). The host repository path is not published here.

## Build / install (acceptance)

```powershell
cd projects/bgxiong-h3-face-refine-adapter
powershell -NoProfile -ExecutionPolicy Bypass -File .\Build.ps1 -Jobs 2
# Then Package-Install.ps1; prepare runtime with companion\scripts\prepare-runtime.ps1
```

See the project [README.md](../../../projects/bgxiong-h3-face-refine-adapter/README.md), [harness/OPS-CHECKLIST.md](../../../projects/bgxiong-h3-face-refine-adapter/harness/OPS-CHECKLIST.md), and install envelope notes beside accepted builds.
