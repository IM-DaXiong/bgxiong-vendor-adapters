# Upgrade notes: vendor adapter query terminalFailure 1.2.0

This is a payload contract tightening. WIT stays `bgxiong:vendor-adapter@1.1.0` (no ABI change). Rebuild official guests so they emit the new query field.

## Must change

- `query` `failed` / `cancelled` / `expired` **must** include non-empty `terminalFailure`.
- `progressText` is running/queued only.
- Host rejects a bare `failed` as `adapterBadOutput` + `ADAPTER_QUERY_TERMINAL_FAILURE_MISSING` (upgrade the package). No `progressText` / `vendorMessage` fallback.

## Old in-flight packages

An old package that still returns `{ "status": "failed" }` will fail locally as "adapter package too old". The host does not invent a vendor reason and does not auto-resubmit.

## Auth modes (platform-neutral)

`volc-signature-v4` is removed. Guests that need HMAC over a canonical request plus a credential header must declare `hmac-sha256-credential`. The unimplemented reserved mode is `http-sigv4` (not a vendor name). There is no alias fallback.
