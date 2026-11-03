# Companion tracking test fixtures (P02)

## Isolated media for multi-keyframe probe

The real SAM2 probe (`../probe_sam2_multikeyframe.py`) **synthesizes** a short
8-frame 64x64 moving-square tensor in-process. No checked-in binary media is
required for the default probe path.

## Optional external short clip

If you want to re-run the probe against a real file later (P12/P16), place a
short progressive clip here, for example:

- `fixtures/short_moving_subject.mp4` (2–4s, constant fps, single clear subject)

and point an env override / future probe flag at that path. Keep files small;
do not commit large production footage.

## How to run

Unit contract tests (no GPU / no SAM2 weights):

```text
python -m unittest packages.workflow-adapters.wan22-animate.companion.tests.test_tracking
```

From this directory:

```text
python -m unittest test_tracking
```

Real upstream predictor probe (Comfy venv + CUDA + sam2 weights):

```text
"<ComfyUI>\.venv\Scripts\python.exe" probe_sam2_multikeyframe.py
```

Env overrides: `BGX_COMFY_ROOT`, `BGX_SAM2_WEIGHTS`, `BGX_SAM2_CFG`.