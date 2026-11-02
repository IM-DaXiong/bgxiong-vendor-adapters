# Video Target Keyframes profile v1 (ideo-target-keyframes-v1)

Protocol remains 2 / WIT 2.0.0. **No** new transport operations or WIT methods. Reuses submit/query/cancel, managed media, checkpoints, and structured data + media outputs.

Packages must declare 
equiredFeatures: ["video-target-keyframes-v1"]. Unsupported hosts reject before execution; they must not silently fall back to single-frame picking. This profile sits **alongside** media-target-selection-v1 without renaming or breaking the analysis-based face contract. Ordinary point picking keeps using the older profile.

## Neutral semantics

- One target, multiple correction keyframes inside one continuous clip (positive/negative points + at most one box).
- **No** analysisId / candidateId; face detection is not a prerequisite.
- Public shapes describe object selection, direction, progress facts, mask receipts, and generation freeze only.
- Public schemas **must not** contain SAM/Comfy/vendor node names, disk paths, or platform error codes.
- Coordinates: source-frame pixels after display-orientation correction; boxes are half-open; times in SDK inputs are **clip-relative milliseconds** (host subtracts 
angeStartMs once).

## Package declaration: x-targetTracking

Declared on the package inputSchema and **explicitly allowed/validated** by SDK schema-policy (consumers must not ignore it as an unknown keyword). See declaration.schema.json.

Rules:

1. Hosts call only the exact **roles → operationRef** bindings (operationId + operationVersion + inputSchemaDigest). Do not guess by operationId substrings such as preview/nalyze.
2. 	rackingPreview must not be presented as the final generation model.
3. Tools/directions absent from the declaration must not be shown or submitted.
4. centroidSummary, when present, is a result summary only — **not** an origin trajectory.
5. Whether reference/prompt edits invalidate tracking comes only from 	rackingDependencies / 
eferenceChangeInvalidatesTracking; hosts must not infer model semantics.

## Deliverables in this directory

| File | Role |
|------|------|
| declaration.schema.json | Closed x-targetTracking declaration |
| selection.schema.json | Correction set (keyframes / points / box) |
| 	racking.input.schema.json | Tracking request including direction |
| progress.schema.json | Per-direction progress facts |
| 	racking-receipt.schema.json | Completion receipt (covered/lost/uncomputed/skip + ready + opaque maskArtifact) |
| rame-page.schema.json | Continuable per-frame page |
| generation-freeze.schema.json | Generation freeze bound to receipt + mask |
| ixtures/receipts.json | Validator fixtures |

## Host negotiation sequence

1. UI draft → host freezes selection → submit 	rackingPreview by role  
2. Consume progress/receipt; frame preview via managed media or rame-page  
3. Edits change selectionDigest → prior receipt expires immediately → explicit re-track  
4. On confirm, build GenerationFreeze → call generate by role; adapter reads the **same** confirmed mask  

Selection without a receipt stays "selected, not yet tracked"; hosts must not forge whole-clip completion or 
eady=true.

## Registration

Registered through validator SchemaRegistry::public (same pattern as media-target-selection-v1 / image-projection-v1). Generate types only via existing generators; never hand-edit generated files. protocolVersion / WIT unchanged.
