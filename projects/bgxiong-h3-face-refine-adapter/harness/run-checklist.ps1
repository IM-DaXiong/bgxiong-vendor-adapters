param(
  [ValidateSet('analyze','preview','refine','all')]
  [string]$Op = 'all'
)
$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
Set-Location $Root
if ($Op -eq 'analyze' -or $Op -eq 'all') { python .\harness\ops\analyze.py; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE } }
if ($Op -eq 'preview' -or $Op -eq 'all') { python .\harness\ops\preview.py; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE } }
if ($Op -eq 'refine' -or $Op -eq 'all') { python .\harness\ops\refine.py; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE } }
