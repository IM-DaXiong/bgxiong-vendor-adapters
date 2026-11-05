[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Cli,
    [Parameter(Mandatory)][string]$Worker
)
$ErrorActionPreference = 'Stop'
if (-not [IO.Path]::IsPathFullyQualified($OutputDirectory)) { throw 'OutputDirectory must be absolute.' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'OutputDirectory already exists; preserve earlier artifacts.' }
$repo = 'L:/00Dev/bgxiong-ai-story'
$sdk = Join-Path $repo 'packages/vendor-adapter-contract/v2'
foreach ($file in @($Cli, $Worker)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing tool: $file" }
}
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$sdkOut = Join-Path $OutputDirectory 'vendor-adapter-v2'
& robocopy $sdk $sdkOut /E /XJ /XD target node_modules .git .bgx-sdk __pycache__ /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "Public SDK copy failed: $LASTEXITCODE" }
New-Item -ItemType Directory -Path (Join-Path $sdkOut 'bin') -Force | Out-Null
Copy-Item -LiteralPath $Cli -Destination (Join-Path $sdkOut 'bin/bgx-adapter-v2.exe')
Copy-Item -LiteralPath $Worker -Destination (Join-Path $sdkOut 'bin/bgx-adapter-v2-worker.exe')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'sample-source') -Destination $OutputDirectory -Recurse
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../tools/Build-ComfyExample.ps1') -Destination $OutputDirectory
$newScript = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '../tools/New-ComfyExample.ps1')).Replace('bgx-comfy-wan22-example','bgx-comfy-ltx23-ia2v-example')
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'New-ComfyExample.ps1'), $newScript, [Text.UTF8Encoding]::new($false))
foreach ($name in @('README.zh-CN.md','request-ia2v.json','Invoke-Ltx23Ia2vExample.ps1')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $OutputDirectory
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'sample-source/workflow/source.json') -Destination (Join-Path $OutputDirectory 'ltx23-ia2v.source.json')
Write-Output "LTX2.3 IA2V author kit: $OutputDirectory"
