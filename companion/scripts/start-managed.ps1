param(
  [Parameter(Mandatory = $true)][string]$RuntimeRoot,
  [Parameter(Mandatory = $true)][string]$ComfyPython,
  [Parameter(Mandatory = $true)][string]$ComfyMain,
  [Parameter(Mandatory = $false)][int]$Port = 8189,
  [Parameter(Mandatory = $false)][string]$Listen = "127.0.0.1"
)
$ErrorActionPreference = "Stop"
$Extra = Join-Path $RuntimeRoot "extra_model_paths.yaml"
if (-not (Test-Path $Extra)) { throw "Missing $Extra — run prepare-runtime.ps1 first" }
$argList = @(
  $ComfyMain,
  "--listen", $Listen,
  "--port", "$Port",
  "--extra-model-paths-config", $Extra
)
Write-Host "START_COMFY python=$ComfyPython port=$Port extra=$Extra"
# Caller/manager owns process lifetime; this script documents the isolated launch contract.
& $ComfyPython @argList
