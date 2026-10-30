# A05 runnable ops checklist

Upstream locked: `d8521d14fe0d721d80cd9417fff5a559cbc21aba` (v1.1.2). Live user media stays local (env paths only).

## Prerequisites

1. Prepare isolated runtime (does **not** overwrite global `custom_nodes`):

```powershell
$Runtime = 'L:\00Dev\runtimes\h3-face-refine-1.1.2'
New-Item -ItemType Directory -Force -Path $Runtime | Out-Null
powershell -File .\companion\scripts\prepare-runtime.ps1 -RuntimeRoot $Runtime
```

2. Start Comfy with package overlay (example port 8189):

```powershell
powershell -File .\companion\scripts\start-managed.ps1 `
  -RuntimeRoot $Runtime `
  -ComfyPython '<Comfy python.exe>' `
  -ComfyMain '<Comfy main.py>' `
  -Port 8189
```

3. Probe:

```powershell
powershell -File .\companion\scripts\probe.ps1 -BaseUrl http://127.0.0.1:8189
```

4. Env for harness:

```powershell
$env:H3_FACE_COMFY_BASE = 'http://127.0.0.1:8189'
$env:H3_FACE_SOURCE_VIDEO = '<local 107-frame mp4 absolute path>'
$env:H3_FACE_APPEARANCE_REF = '<local appearance still>'
$env:H3_FACE_IDENTITY_REF = '<local identity still>'
$env:H3_FACE_OUT_DIR = (Resolve-Path .\verification\out).Path
```

## Three ops

```powershell
python .\harness\ops\analyze.py
# Edit verification\out\selection.json → pick distant woman candidateId
python .\harness\ops\preview.py
# Human-review tracking; only then:
# $env:H3_FACE_SUBMIT_PREVIEW = '1'   # when weight map ready
python .\harness\ops\refine.py
# $env:H3_FACE_SUBMIT_REFINE = '1'    # after preview OK
```

## A05 status expectations

| Step | Status when Comfy down / weights map incomplete |
|------|--------------------------------------------------|
| Package + schemas + isolation scripts | DONE |
| analyze against live `/h3_facerefine/scan` | READY (needs Comfy+nodes) |
| preview bindings face_pick | READY |
| preview/refine full queue + audio check | GATED until managed weight filenames + Comfy up |

Do not commit private user media. Synthetic fixtures under `fixtures/synthetic/`.
