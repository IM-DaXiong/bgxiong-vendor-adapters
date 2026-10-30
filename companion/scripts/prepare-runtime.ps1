param(
  [Parameter(Mandatory = $true)][string]$RuntimeRoot,
  [Parameter(Mandatory = $false)][string]$SharedModelsRoot = "",
  [Parameter(Mandatory = $false)][string]$ComfyRoot = ""
)
$ErrorActionPreference = "Stop"
$Companion = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not (Test-Path (Join-Path $Companion "companion\upstream\ComfyUI-H3-FaceRefine\nodes.py"))) {
  # When invoked from companion/scripts, companion root is parent of scripts
  $Companion = Split-Path $PSScriptRoot -Parent
}
$Src = Join-Path $Companion "upstream\ComfyUI-H3-FaceRefine"
$DstRoot = Join-Path $RuntimeRoot "custom_nodes_overlay\ComfyUI-H3-FaceRefine"
New-Item -ItemType Directory -Force -Path $DstRoot | Out-Null
robocopy $Src $DstRoot /E /NFL /NDL /NJH /NJS /nc /ns /np | Out-Null
# Write runtime-local extra paths that point ONLY at this overlay (never shared custom_nodes)
$Extra = @"
h3_face_refine_adapter:
  base_path: $($RuntimeRoot.Replace('\','/'))/
  custom_nodes: custom_nodes_overlay/
"@
$ExtraPath = Join-Path $RuntimeRoot "extra_model_paths.yaml"
Set-Content -LiteralPath $ExtraPath -Value $Extra -Encoding UTF8
$marker = @{
  prepared = $true
  upstreamCommit = (Get-Content (Join-Path $Companion "..\locks\upstream-comfyui-h3-facerefine.sha") -ErrorAction SilentlyContinue)
  overlay = $DstRoot
  extraModelPaths = $ExtraPath
  sharedModelsRoot = $SharedModelsRoot
  note = "Did not modify global ComfyUI custom_nodes"
} | ConvertTo-Json
Set-Content -LiteralPath (Join-Path $RuntimeRoot "prepare-receipt.json") -Value $marker -Encoding UTF8
Write-Host "PREPARE_OK runtime=$RuntimeRoot overlay=$DstRoot"
