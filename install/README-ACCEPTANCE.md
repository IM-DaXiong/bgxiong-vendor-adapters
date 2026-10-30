# Face-refine adapter install (human acceptance)

Built from sibling package bgxiong-h3-face-refine-adapter with toolchain **1.95.0**, target wasm32-wasip2, CARGO_TARGET_DIR=target-face-b.

## Package layout

```text
install/
  adapter-install.json          # app distribution envelope (auth=none, private LAN)
  package/
    adapter.wasm
    manifest.json               # pluginId local.thirdparty.h3-face-refine @ 1.0.0-a04
    manifest.sha256
```

Do **not** point this at real `K:\bgxiong-ai-story\data\story.db`. Use INT worktree or a throwaway app-data root.

## 1) Prepare managed Comfy overlay (no global custom_nodes)

```powershell
$Runtime = 'L:\00Dev\runtimes\h3-face-refine-1.1.2'
New-Item -ItemType Directory -Force -Path $Runtime | Out-Null
powershell -NoProfile -ExecutionPolicy Bypass -File .\companion\scripts\prepare-runtime.ps1 -RuntimeRoot $Runtime
```

Managed runtime was prepared once on DESKTOP-4OVG8RB at that path (overlay + extra_model_paths.yaml). Re-run prepare after upstream lock bumps.

## 2) Start managed Comfy (user starts; do not block waiting)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\companion\scripts\start-managed.ps1 `
  -RuntimeRoot $Runtime `
  -ComfyPython '<path-to-comfy-python.exe>' `
  -ComfyMain '<path-to-ComfyUI\main.py>' `
  -Port 8189
powershell -NoProfile -ExecutionPolicy Bypass -File .\companion\scripts\probe.ps1 -BaseUrl http://127.0.0.1:8189
```

Default adapter-install.json endpointBaseUrl is http://127.0.0.1:8189. Edit hostConfig for 8188 or LAN 192.168.18.8.

## 3) Install into INT / product (GUI or Creator)

Install the **outer** folder:

`L:\00Dev\bgxiong-h3-face-refine-adapter\install`

via GUI / F1 / MCP `comfy.workflow.adapterPackage.inspect` then `workflow.adapterPackage.install` (monorepo guide docs/dev/guides/20261028-GUIDE-SDKv2适配包安装声明与账号凭据.md). Auth source is `none` — no API key prompt.

Advanced: native SDK package dir is `install\package` (wasm + manifest only).

Prefer documenting how the user starts INT (`L:\00Dev\wt-face-int`, branch `codex/face-refine-int`) rather than a bot full product compile. Optional low-jobs check only if policy allows: CARGO_TARGET_DIR=L:\00Dev\wt-face-int\target-face-int.

## 4) Optional harness without full product compile

See harness/OPS-CHECKLIST.md. Env: H3_FACE_COMFY_BASE, local media paths only.

## Digests (this build)

- adapter.wasm sha256: b1ffddbd60a05b6b4a8bd5cc1efe85d0e74823d0d0608d1b61990883bec45da0
- manifest.json sha256: b2e5086b60bd34900cae7c2198ed72c23656bc90e3722e9e2b4915814a87bf5d
- size: 868480 bytes
