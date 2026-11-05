# T22 remainder: H1 version bump + CDN / offline distribution

Date: 2026-10-26 (Asia/Shanghai)

## Scope (authorized)
- Base H1 install package **version bump** + CDN / offline distribution only.
- **Not** T21 H2 freeze, **not** D pack, **not** putting C/stream/batch into base kit.

## Pull
`19b68cdf6` → `19b68cdf6` (Already up to date). Worktree clean before bump.

## Version
- `scripts/bump-version.ps1 0.6.1026`
- **old → new:** `0.6.1102` → `0.6.1026` (release-date MMDD = 1026)
- Touched: root + src-tauri Cargo.toml/lock, ui package.json/lock, packages/release-scope/current.json

## Offline kit refresh
```powershell
powershell -NoProfile -File scripts/publish-breeze-h1-kit.ps1 -Force -AttemptUpload
```
(also rebuilds via `build-breeze-h1-offline-kit.ps1`)

| Item | Value |
|------|-------|
| Zip | `packages/tts-adapters/breeze/dist-offline-h1/breeze-tts-h1-1.0.0-a.zip` |
| Sha256 | `6dcfb05e6773f2924c66e392f4304c68eeee2ea2172761a17d741f27e60ebd13` |
| Bytes | 371503 |
| A wasm | `40570a1ead1b92bc233d1650d9feeeff518eede5b6c75b20fbcc8bcd60074a3f` (frozen) |
| includesCPack | false |
| includesH2 | false |
| cdnUploaded | **false** (honest) |

Also staged:
- `dist/offline/tts/breeze-h1/` (build out; gitignored)
- `data/offline-drop/BreezeTTS/` (product scan root; under `data/` gitignored)
- `D:\00Dev\UPLOAD_TEMP\client\ai-story\runtimes\BreezeTTS\`
- `runtimes/manifests/breeze-tts-h1-1.0.0-a.json`
- `src-tauri/resources/runtime-manifests/breeze-tts-h1-1.0.0-a.json`

## CDN layout
- `runtimes/cdn-object-layout.json`: stem `breeze-tts-h1` → `BreezeTTS`; prefix `breeze-tts-h1-`
- Contract URLs (not downloadable until ops upload):
  - https://r.bgxiong.com/client/ai-story/runtimes/BreezeTTS/breeze-tts-h1-1.0.0-a.zip
  - https://r.bgxiong.com/client/ai-story/runtimes/BreezeTTS/breeze-tts-h1-1.0.0-a.json
  - https://r.bgxiong.com/client/ai-story/runtimes/BreezeTTS/breeze-tts-h1-20261026.json

## CDN attempt
- Existing publish path: prepare + print `curl -T` (no in-repo auto-uploader).
- Attempted anonymous HTTP PUT → **404 Not Found** on all three objects.
- Missing on this machine: `rclone` / `coscli` / `ossutil` / `aws` / `wrangler`; env `COS_*` / `AWS_*` / `RCLONE_CONFIG` / `BGXIONG_CDN_UPLOAD` empty.
- `verify-runtime-cdn.ps1 -Runtime breeze-h1` → FAIL (no CDN manifest candidate) — expected until ops upload.

### Next human step
1. Sync `D:\00Dev\UPLOAD_TEMP\client\ai-story\runtimes\BreezeTTS\` to CDN (same path under `r.bgxiong.com`), **or** run the three `curl -T` lines with write credentials.
2. `powershell -NoProfile -File scripts/verify-runtime-cdn.ps1 -Runtime breeze-h1`
3. Only then set `cdnUploaded=true` in meta (re-run publish with verified upload, or edit after verify PASS).

## Installer EXE
- Version SSOT bumped to `0.6.1026`.
- Full Tauri/NSIS rebuild **not** run in this slice (avoid fighting user cargo lock; no CDN write creds for updater publish either).

## NOT done
- T21 H2 freeze
- D pack
- C/stream/batch inside base H1 kit
- Verified CDN upload (`cdnUploaded` stays false)
- Fresh NSIS installer EXE for 0.6.1026