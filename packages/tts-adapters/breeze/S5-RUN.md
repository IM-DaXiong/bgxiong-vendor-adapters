# S5: H1 offline kit + author/handoff (T22–T24)

## Pull
`efb78fefd` → `efb78fefd` (Already up to date)

## Scope
- **H1 base only** (A-pack `1.0.0-a`).
- C-pack stream/batch **not** in base kit (T21 H2 **not** frozen — do not claim H2).

## T22 Build (kit)
```powershell
powershell -NoProfile -File scripts/build-breeze-h1-offline-kit.ps1 -Force
# or:
powershell -NoProfile -File scripts/build-tts-offline-bundle.ps1 -Runtime breeze -Force
```
Outputs under `dist/offline/tts/breeze-h1/` (+ tracked `packages/tts-adapters/breeze/dist-offline-h1/`).

## T22 remainder (version + CDN / offline) — 2026-10-26
See **`T22-RUN.md`**.
```powershell
powershell -NoProfile -File scripts/bump-version.ps1 0.6.1026
powershell -NoProfile -File scripts/publish-breeze-h1-kit.ps1 -Force -AttemptUpload
powershell -NoProfile -File scripts/verify-runtime-cdn.ps1 -Runtime breeze-h1
```
- App version: `0.6.1102` → `0.6.1026`
- Kit sha256: `6dcfb05e6773f2924c66e392f4304c68eeee2ea2172761a17d741f27e60ebd13`
- `cdnUploaded=false` until ops upload + verify PASS (no fake downloadable links).

## T23 Author
See kit `AUTHOR.md` and `packages/vendor-adapter-contract/v2/templates/tts-adapter/`.
Authors must not need private `src-tauri` sources.

## T24 Handoff
See `packages/tts-adapters/breeze/HANDOFF.md`.