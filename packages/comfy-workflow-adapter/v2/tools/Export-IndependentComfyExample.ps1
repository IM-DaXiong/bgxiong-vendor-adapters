[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$SourceDirectory,
    [Parameter(Mandatory)][string]$ProjectDirectory,
    [Parameter(Mandatory)][string]$SdkDirectory,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Cli,
    [Parameter(Mandatory)][string]$Worker
)
$ErrorActionPreference = 'Stop'
if (-not [IO.Path]::IsPathFullyQualified($OutputDirectory)) { throw 'OutputDirectory must be absolute.' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Preserve the previous output; use a new directory.' }
$repo = 'L:/00Dev/bgxiong-ai-story'
$sdk = Join-Path $repo 'packages/vendor-adapter-contract/v2'
foreach ($file in @($Cli, $Worker, (Join-Path $SourceDirectory "$ProjectDirectory/manifest.json"))) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing input: $file" }
}
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$sdkOut = Join-Path $OutputDirectory $SdkDirectory
& robocopy $sdk $sdkOut /E /XJ /XD target node_modules .git .bgx-sdk __pycache__ /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "Public SDK copy failed: $LASTEXITCODE" }
New-Item -ItemType Directory -Path (Join-Path $sdkOut 'bin') -Force | Out-Null
Copy-Item -LiteralPath $Cli -Destination (Join-Path $sdkOut 'bin/bgx-adapter-v2.exe')
Copy-Item -LiteralPath $Worker -Destination (Join-Path $sdkOut 'bin/bgx-adapter-v2-worker.exe')
$projectOut = Join-Path $OutputDirectory $ProjectDirectory
& robocopy (Join-Path $SourceDirectory $ProjectDirectory) $projectOut /E /XJ /XD target .bgx-sdk __pycache__ /XF adapter.wasm manifest.sha256 /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "Author project copy failed: $LASTEXITCODE" }
# Export only current author instructions. Historical runs remain in project records.
foreach ($name in @('README.md', 'README.zh-CN.md', 'FINAL-ACCEPTANCE.zh-CN.md', 'Build-Flux2.ps1', 'Invoke-Flux2.ps1', 'Build-Continuum.ps1', 'Invoke-Continuum.ps1', 'request-two-reference.json', 'request-continuum.json', 'host-config.app.json')) {
    $source = Join-Path $SourceDirectory $name
    if (Test-Path -LiteralPath $source -PathType Leaf) { Copy-Item -LiteralPath $source -Destination $OutputDirectory }
}
Write-Output "Independent author kit: $OutputDirectory"
