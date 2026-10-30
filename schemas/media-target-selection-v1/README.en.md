# Media Target Selection profile v1

Protocol remains 2 / WIT 2.0.0. No new transport ops. Reuses `videoFaceRefine` slot + submit/query/cancel.

Packages must declare `requiredFeatures: ["media-target-selection-v1"]`. Business op IDs are package-declared.

See sibling schemas for source identity, analysis receipt, selection, tracking, freeze, and analyze/preview/refine IO. Fixtures in `fixtures/receipts.json`.

Registered via validator `SchemaRegistry::public` like image-projection-v1. Not yet in generate_contracts wire NAMES.

## Frozen field pointers (T02/T03 · 2026-10-30)

Single names only — do **not** accept both `references` and `referenceMediaIds`, or both `params` and `userParams`.

- Appearance refs: `/freeze/referenceMediaIds`
- Prompt: `/freeze/prompt`
- Package knobs (denoise, face strengths, SAM, scaleMode, identity): `/freeze/userParams/*`
- Analyze detection: `/detectionParams/*`
- Stages: analyze→`/source`+receipt; preview→analysis+selection; refine→freeze+selection[+tracking]

See `fixtures/receipts.json` for valid three-stage originals and invalid cross-source / missing-ready / alias cases.
