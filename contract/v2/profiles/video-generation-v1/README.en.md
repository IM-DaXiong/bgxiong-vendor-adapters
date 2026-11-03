# video-generation-v1: public bidirectional video-generation parameter contract

This directory is the public consumption contract for `video.generate`: how adapters declare workbench parameters, and which standard fields are submitted after user confirmation. The protocol remains V2 / WIT 2.0.0. This directory does **not** add a runtime envelope, WIT fields, a combination interpreter, or an SDK execution version.

Authoritative shapes:

- Capability/operation: [`schemas/capability-contract.schema.json`](../../schemas/capability-contract.schema.json)
- This example input: [`input.example.schema.json`](input.example.schema.json)
- UI controls: [`schemas/ui-schema.schema.json`](../../schemas/ui-schema.schema.json)
- Media bytes: existing controlled media channel; only `mediaId` appears in input

The SDK publishes field contracts, reads declarations, forwards standard values, and reuses existing execution messaging. The SDK does **not** understand vendor business rules, does not validate vendor size combinations, does not compute vendor frame grids, and does not repair parameters for adapters. Platform limits, graphs, URLs, and error-code mapping stay in out-of-repo adapters.

## 1 Bidirectional flow

1. **Adapter → SDK → workbench**: declare supported parameters, types, units, defaults, enums/ranges, fixed values, and media roles/counts via `inputSchema` / `uiSchema` / `mediaSlots` / `execution` / `outputSlots`.
2. **User confirm → SDK → adapter**: send the selected operation identity, mode, specs, prompt, media, and other generation parameters; fixed parameters use declared values.
3. **Adapter → cloud**: map standard fields to platform fields and run business validation.
4. **Adapter → SDK → app**: return acceptance, progress, errors, and video outputs on the existing execution contract.

Undeclared fields are unsupported; defaults must not impersonate a supported range. Combination rules beyond the declaration are adapter responsibility with explicit errors. Valid user choices are not overwritten by defaults. Errors must not silently switch models, mutate parameters, or resubmit.

## 2 Field checklist (mark each supported/unsupported when implementing)

Field names, types, and units below are the public profile contract. Hosts may present options/defaults and basic type/enum/range hints from declarations; authors must not be left to guess names from free-form JSON.

| Field | Unit / shape | Declaration | Submit rules |
| --- | --- | --- | --- |
| Model/operation identity | execution envelope | `modelId`, `operationId`, `operationVersion`, package version, `inputSchemaDigest` | Display names are not IDs; switching operations carries full identity |
| `inputMode` | string | `const` on that operation's inputSchema | Submit the mode of the selected operation; do not infer from image count |
| `prompt` | string | usually required | Full user-confirmed text; keep newlines and material-reference text; no truncation/rewrite |
| `negativePrompt` | string | omit = unsupported; declared non-required = optional | Show/submit only when supported; explicit clear sends `""`; omit when unsupported |
| `durationSeconds` | integer seconds | `const` / `enum` / min+max(+`multipleOf`); `default` init-only | Fixed values must still be submitted; this consumer round uses whole seconds; SDK does not compute vendor frames |
| `fps` | frames/second | same patterns | Submit selected or declared fixed value; do not rewrite with playback fps |
| `aspectRatio` | string | package enum/fixed | Declared values including `9:5` must not be dropped by a local common-ratio whitelist; aspect↔size matching is adapter-side |
| `width` / `height` | pixels | fixed/enum/range + step | Send resolved/generic pixel values as-is; do not synthesize via platform formulas |
| `resolution` | tier id | named enum | Submit package tier id, never UI label text |
| `resolutionValue` | `{kind:"megapixels",value}` | only when declared | Unit MP; mutually exclusive with pixels and named tiers |
| `seed` | integer | type/default/range | Pass user value; no SDK randomization/truncation; out-of-exact-range is a structural error |
| Reference media | `mediaId` arrays | `mediaSlots` role/pointer/kind/counts | First/last frame, ordered reference images/audio/video; empty/remove/reorder follow the workbench; bytes use the controlled channel |
| `advanced.audioUsage` | string enum | required when reference audio exists | Distinct from “output contains audio” |
| Other `advanced.*` | closed scalars | `additionalProperties:false` with defaults | Send when the workbench has a value; `false`, `0`, `""`, and `[]` are legal |
| Host `vendorFeatureDraft` | draft only | no separate SDK field | Must land on declared `advanced` (or other declared scalars); never drop silently; never also send a duplicate `vendorFeatures` bag |

### 2.1 Exactly one size representation

Pixel size (`width`/`height`), named tier (`resolution`), and megapixels (`resolutionValue`) are different representations. A package must accept exactly one. A request must not mix leftover drafts from another representation.

[`input.example.schema.json`](input.example.schema.json) uses **pixel size**. `$defs.NamedResolutionExample` and `$defs.MegapixelResolutionExample` illustrate the other two declaration styles and are not root fields of this example.

### 2.2 Fixed / enum / range

- **Fixed**: `const` (optional `default` for init). Locked controls still submit the fixed value.
- **Enum**: `enum` + `default`.
- **Range**: `minimum` / `maximum` / optional `multipleOf` + `default`.
- `default` initializes the form only; formal submit does not server-fill defaults or silently delete fields.

The example covers fixed `fps`, enum `durationSeconds`, ranged `width`/`height`/`seed`, and optional `negativePrompt`.

### 2.3 Media roles (never infer from counts)

Each `mediaSlots` entry must state its role. Recommended role names:

| role | mediaKind | typical inputPointer | meaning |
| --- | --- | --- | --- |
| `firstFrame` | `image` | `/firstFrame` | first frame |
| `lastFrame` | `image` | `/lastFrame` | last frame |
| `referenceImages` | `image` | `/referenceImages` | ordered reference images |
| `referenceAudios` | `audio` | `/referenceAudios` | ordered reference audio |
| `referenceVideos` | `video` | `/referenceVideos` | ordered reference video |

Input fields are arrays of `{mediaId}` objects. One operation has one `inputMode`; one model may expose many operations. Published fields are not the same as “fully implemented in every host consumer”; unsupported modes must not be registered as available.

### 2.4 Execution and outputs

Reuse existing `execution` / `outputSlots`. Do not advertise sync/async/cancel/resume/output forms that are not implemented.

### 2.5 App-owned fields stay out of cloud generation input

Node/workspace IDs, history, save locations, window layout, media memory, confirmation state, batch counts, local orchestration, and post-processing remain app-owned. Adapters receive the confirmed final `prompt` and §2 standard fields only.

## 3 What authors provide / what submit delivers

See the Chinese README for the full neutral capability fragment and sample submit input (same JSON). Validation approach with the existing SDK:

1. Register the package schema with SchemaRegistry (digest, local `$ref`, schema-policy keywords).
2. Strictly validate submit `input`: missing required, additional properties, enum/range errors fail; no default filling; no silent field deletion.
3. Align `mediaSlots` with input arrays and leased `mediaId`s; keep `execution`/`outputSlots` honest.

## 4 Related public docs

- Authoring: [AUTHORING.zh-CN.md](../../AUTHORING.zh-CN.md)
- Host integration: [HOST-INTEGRATION.zh-CN.md](../../HOST-INTEGRATION.zh-CN.md)
- Developer preview: [DEVELOPER-PREVIEW.zh-CN.md](../../DEVELOPER-PREVIEW.zh-CN.md)

An independent author reading only this public directory and the public SDK tree can implement capability declarations and consume submit values; platform mapping stays inside the author package.
