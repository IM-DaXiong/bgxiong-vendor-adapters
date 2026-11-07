# Schema alignment (Lane A)

Public profile in this repository:

[media-target-selection-v1](../../../contract/v2/profiles/media-target-selection-v1/)

This package mirrors those files under [media-target-selection-v1/](media-target-selection-v1/) for independent-author builds. The host repository copy is not published here and was not re-read for this note.

## Frozen pointers (2026-10-30)

- Appearance refs: `/freeze/referenceMediaIds` (not `references`)
- Knobs: `/freeze/userParams/{denoise,smallFaceStrength,largeFaceStrength,samEnabled,scaleMode,identityMediaIds}` (not `/freeze/params` or `/sam/enabled`)
- Prompt: `/freeze/prompt`
- Analyze detection: `/detectionParams/*`
- `requiredFeatures: ["media-target-selection-v1"]` on capability + each operation execution

scaleMode: upstream supports `absolute_px` | `relative_to_clip` — declared and passed by this package.
