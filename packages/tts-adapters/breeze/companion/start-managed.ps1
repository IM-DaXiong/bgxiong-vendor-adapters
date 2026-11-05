# The package owns its SoX executable; never depend on the user's PATH.
param([int]$Port = 0)
$ErrorActionPreference = 'Stop'
$soxRoot = Join-Path $PSScriptRoot 'sox'
if (-not (Test-Path -LiteralPath (Join-Path $soxRoot 'sox.exe'))) { throw 'Breeze SoX resource is not prepared' }
$env:PATH = $soxRoot + ';' + $env:PATH
& (Join-Path $PSScriptRoot 'start-sidecar.ps1') -Port $Port @args
exit $LASTEXITCODE
