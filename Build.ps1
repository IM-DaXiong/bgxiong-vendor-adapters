param([int]$Jobs = 2)
$ErrorActionPreference = 'Stop'
$Root = $PSScriptRoot
$env:CARGO_TARGET_DIR = Join-Path $Root 'target-face-b'
Write-Host "CARGO_TARGET_DIR=$env:CARGO_TARGET_DIR jobs=$Jobs"
rustup run 1.95.0 cargo build --manifest-path (Join-Path $Root 'Cargo.toml') --release --target wasm32-wasip2 -j $Jobs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$wasm = Get-ChildItem (Join-Path $env:CARGO_TARGET_DIR 'wasm32-wasip2\release') -Filter '*.wasm' -Recurse | Select-Object -First 5
$wasm | ForEach-Object { Write-Host "WASM" $_.FullName $_.Length }
