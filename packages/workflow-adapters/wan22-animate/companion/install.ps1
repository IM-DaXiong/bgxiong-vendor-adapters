[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$TargetComfyDir
)
$ErrorActionPreference = 'Stop'
if (-not [IO.Path]::IsPathFullyQualified($TargetComfyDir)) {
    throw 'TargetComfyDir must be an absolute path.'
}
$custom = Join-Path $TargetComfyDir 'custom_nodes'
if (-not (Test-Path -LiteralPath $custom)) {
    throw "Comfy custom_nodes directory not found: $custom"
}
$src = Join-Path $PSScriptRoot 'custom_nodes\BGXAnimateTracking'
if (-not (Test-Path -LiteralPath $src)) {
    throw "Companion source missing: $src"
}
$dest = Join-Path $custom 'BGXAnimateTracking'
# Refuse overwriting unrelated packages; only replace our own companion folder.
if (Test-Path -LiteralPath $dest) {
    $marker = Join-Path $dest '__init__.py'
    if (-not (Test-Path -LiteralPath $marker)) {
        throw "Refusing to overwrite unexpected directory at $dest"
    }
    Remove-Item -LiteralPath $dest -Recurse -Force
}
New-Item -ItemType Directory -Path $dest | Out-Null
Get-ChildItem -LiteralPath $src -File | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $dest $_.Name) -Force
}
# Hash inventory for the installed files
Get-ChildItem -LiteralPath $dest -File | ForEach-Object {
    $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Output ("{0}  {1}" -f $hash, $_.Name)
}
Write-Output "Installed BGXAnimateTracking into $dest"
Write-Output "Restart the target ComfyUI process, then verify object_info for BGXAnimateFrameSequence and BGXAnimateBidirectionalMask."