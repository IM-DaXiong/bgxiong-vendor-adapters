param(
  [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..\..")).Path,
  [string]$PackDir = "",
  [string]$RuntimeRoot = "E:\BreezeTTS2-assets\s1-staging\data\runtimes\tts\breeze\1.0.0",
  [string]$StoreRoot = "E:\BreezeTTS2-assets\s1-staging\data\runtimes\tts\_store",
  [string]$ManagedPython = "E:\BreezeTTS2-assets\s0-staging\python\python.exe",
  [string]$OutHostConfig = ""
)
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
if (-not $PackDir) {
  $PackDir = Join-Path $RepoRoot "packages\tts-adapters\breeze\dist-pack"
  if (-not (Test-Path (Join-Path $PackDir "manifest.json"))) {
    $PackDir = "E:\BreezeTTS2-assets\s1-sdk-prove\pack"
  }
}
if (-not (Test-Path (Join-Path $PackDir "manifest.json"))) {
  throw "pack not found: $PackDir"
}
if (-not (Test-Path (Join-Path $RuntimeRoot "start-sidecar.ps1"))) {
  throw "runtime root missing start-sidecar.ps1: $RuntimeRoot"
}
$hostObj = @{
  speechManaged = @{
    schemaVersion = 1
    runtimeRoot = $RuntimeRoot
    storeRoot = $StoreRoot
    managedPython = $ManagedPython
    healthPath = "/health"
    launcher = "start-sidecar.ps1"
    readyTimeoutSecs = 900
  }
}
if (-not $OutHostConfig) {
  $OutHostConfig = Join-Path $RepoRoot "packages\tts-adapters\breeze\s2-host-config.speechManaged.json"
}
$json = $hostObj | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText($OutHostConfig, $json, [Text.UTF8Encoding]::new($false))
Write-Host "S2 host-config written: $OutHostConfig"
Write-Host "Pack to install in Settings → SDK v2: $PackDir"
Write-Host "Paste the JSON from the host-config file into the install dialog, then click 加入语音模型."
Write-Host "Verify: TTS Studio / 对白 choose speech_vendor_adapter model adapter:local.tts.breeze:sdk-speech-v2.*"
