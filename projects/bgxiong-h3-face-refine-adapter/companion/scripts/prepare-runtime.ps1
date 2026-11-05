param(
  [Parameter(Mandatory = $true)][string]$RuntimeRoot,
  [Parameter(Mandatory = $false)][string]$SharedModelsRoot = "",
  [Parameter(Mandatory = $false)][string]$ComfyRoot = ""
)
$ErrorActionPreference = "Stop"
# This script lives in companion/scripts — companion root is always the parent folder.
$Companion = Split-Path $PSScriptRoot -Parent
$Manifest = Get-Content -LiteralPath (Join-Path $Companion "manifest.json") -Raw | ConvertFrom-Json
$Inventory = Get-Content -LiteralPath (Join-Path $Companion "files.json") -Raw | ConvertFrom-Json
foreach ($item in $Inventory.files) {
  $file = Join-Path $Companion $item.path
  if (-not (Test-Path -LiteralPath $file) -or (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $item.sha256) { throw "Companion inventory mismatch: $($item.path)" }
}
$ExistingReceipt = Join-Path $RuntimeRoot "prepare-receipt.json"
if (Test-Path -LiteralPath $ExistingReceipt) {
  $existing = Get-Content -LiteralPath $ExistingReceipt -Raw | ConvertFrom-Json
  if ($existing.version -ne $Manifest.version) { throw "Use a new version-scoped runtime directory; do not overwrite another companion version" }
}
$Src = Join-Path $Companion "upstream\ComfyUI-H3-FaceRefine"
if (-not (Test-Path (Join-Path $Src "nodes.py"))) {
  throw "Missing upstream nodes at $Src"
}
$DstRoot = Join-Path $RuntimeRoot "custom_nodes_overlay\ComfyUI-H3-FaceRefine"
New-Item -ItemType Directory -Force -Path $DstRoot | Out-Null
robocopy $Src $DstRoot /E /NFL /NDL /NJH /NJS /nc /ns /np | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy failed exit=$LASTEXITCODE" }
$ContractSrc = Join-Path $Companion "custom_nodes\BGXFaceRefineContract"
$ContractDst = Join-Path $RuntimeRoot "custom_nodes_overlay\BGXFaceRefineContract"
New-Item -ItemType Directory -Force -Path $ContractDst | Out-Null
robocopy $ContractSrc $ContractDst /E /XD __pycache__ /NFL /NDL /NJH /NJS /nc /ns /np | Out-Null
if ($LASTEXITCODE -ge 8) { throw "Contract companion copy failed exit=$LASTEXITCODE" }
# Write runtime-local extra paths that point ONLY at this overlay (never shared custom_nodes)
$Extra = @"
h3_face_refine_adapter:
  base_path: $($RuntimeRoot.Replace('\','/'))/
  custom_nodes: custom_nodes_overlay/
"@
$ExtraPath = Join-Path $RuntimeRoot "extra_model_paths.yaml"
Set-Content -LiteralPath $ExtraPath -Value $Extra -Encoding UTF8
$lockPath = Join-Path (Split-Path $Companion -Parent) "locks\upstream-comfyui-h3-facerefine.sha"
$upstream = $null
if (Test-Path -LiteralPath $lockPath) { $upstream = (Get-Content -LiteralPath $lockPath -Raw).Trim() }
$marker = @{
  prepared = $true
  version = $Manifest.version
  upstreamCommit = $upstream
  overlay = $DstRoot
  extraModelPaths = $ExtraPath
  sharedModelsRoot = $SharedModelsRoot
  note = "Did not modify global ComfyUI custom_nodes"
} | ConvertTo-Json
Set-Content -LiteralPath (Join-Path $RuntimeRoot "prepare-receipt.json") -Value $marker -Encoding UTF8
Write-Host "PREPARE_OK runtime=$RuntimeRoot overlay=$DstRoot"
