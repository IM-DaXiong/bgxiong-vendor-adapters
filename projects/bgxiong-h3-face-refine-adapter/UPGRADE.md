# Upgrade guide (authors)

1. Change only this package + managed companion; never the host EXE.
2. Bump `locks/upstream-comfyui-h3-facerefine.sha` and refresh `companion/upstream/`.
3. If picker routes or denoise model wiring change, update `workflows/` and adapter mappings.
4. prepare → probe → atomic activate; never silently fall back.
5. Baseline: `d8521d14fe0d721d80cd9417fff5a559cbc21aba`.
