# Image projection profile v1

Protocol remains 2. Use `image.generate`, a package-owned operationId, existing result.data and checkpoints. Require `image-projection-v1` in the package's requiredFeatures; unsupported hosts reject before execution. No new WIT fields, imports or task envelope.

This feature has no requiredImports and remains in verified package/host negotiation, outside the unchanged Invocation.enabledFeatures WIT enum. Use validator `package::invocation_features` to derive the exact import-enabling subset from the public catalog. Keep the full negotiated set for profile validation and recovery; do not copy every requiredFeatures entry into Invocation.

Projection and file format are independent. Preserve the media broker's decoded MIME, filename, geometry and hash; never rename JPEG/WebP to PNG. Decode HTTP, MediaStat and checkpoint replies using the frozen public DTOs. Required nullable fields must be present; static-image durationMs may be null. MediaStat has no handle: obtain it from the HTTP media payload. Verify the actual incremented checkpoint revision.

Required root input `outputProjection` is a string with a nonempty enum subset of `flat` and `equirectangular360`, and a default belonging to that enum. Even a fixed choice uses a one-element enum. The workbench owns this single public control; extension panels must not duplicate it. Submit the user's frozen choice unchanged. Input mode, reference cardinality, dimensions and other parameters remain declared through the existing schema and mediaSlots, independently of projection. Legacy packages receive no new input fields.

`flat` is an ordinary image. `equirectangular360` is a full, uncropped, monocular horizontal360°/vertical180° equirectangular image; decoded width must equal twice the height. Structure, identity and available geometry are validated, not scene quality or seams. Aspect ratio alone never determines projection.

Completed and succeeded query data follow [output.schema.json](output.schema.json):

```json
{"imageArtifacts":[{"mediaId":"out-1","role":"result","projection":"equirectangular360"}]}
```

Each actual image output has exactly one matching mediaId/role entry. Duplicate, missing, dangling and non-image references are invalid. Every image projection must equal the frozen request. Array position, URL, filename and content hash are not media identity. Invalid geometry fails without cropping, padding or downgrading.

Accepted needs no image data. Queued/running/failed/cancelled/expired query data are `{"imageArtifacts":[]}`, with no image outputs. Success requires at least one image and all declared outputSlots. Streaming, media_chunk and partial images are unsupported in v1; existing ordinary-image streaming is unchanged. Resume the original accepted ID, never resubmit. Cancellation follows the existing declared contract.

Retained inspect preserves original response/dataJson and checkpointResult data; historical handles are not reusable. Retrieval failure is not remote generation failure. Keep the checkpoint, original bytes and error receipt; retry the original ID only when conditions permit. Custom usage metric and unit must both use the package namespace; never invent usage. Correlated SDK trace stages identify managed-port, result-validation and receipt-validation failures without private URLs, credentials or raw business content.

This consumer/author field convention does not replace bindingId/profileId or define a std/ operation. The example can use one reference slot with minCount=maxCount=1; silently selecting the first of multiple inputs is forbidden.

2.1.1: the media broker fully decodes image pixels within allocation limits. JPEG uses the existing locked decoder in strict mode; truncated scans are rejected, and original bytes are preserved without transcoding.
