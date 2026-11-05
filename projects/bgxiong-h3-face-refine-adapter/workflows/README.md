# Package workflows

- `preview.track.api.json` — analyze→selection→track preview (H3FaceSelect + H3FaceTrackCrop preview=true)
- `refine.manual.api.json` — full refine template (must keep SAM optional; default rectangular mask path)

UI example workflows from upstream 1.1.2 are vendored under `companion/upstream/ComfyUI-H3-FaceRefine/example_workflows/` for author reference. Runtime uses API-format graphs assembled by the adapter/harness.
