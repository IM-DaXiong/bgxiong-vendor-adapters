# Export one source-derived author kit. Run with PowerShell 7; never changes the SDK.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Cli,
    [Parameter(Mandatory)][string]$Worker
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/resolve-development-roots.ps1')
$roots = Get-AdapterDevelopmentRoots $PSScriptRoot
$repo = $roots.HostRoot
$sdk = Join-Path $repo 'packages/vendor-adapter-contract/v2'
$source = Join-Path $roots.AdapterRoot 'packages/workflow-adapters/wan22-animate'
$workflow = Join-Path $repo 'ComfyUI-API/wan2.2/video_wan2_2_14B_animate.json'
$out = [IO.Path]::GetFullPath($OutputDirectory)
if (-not [IO.Path]::IsPathFullyQualified($OutputDirectory)) { throw 'OutputDirectory must be absolute.' }
if (Test-Path -LiteralPath $out) { throw 'OutputDirectory must not exist; existing artifacts are preserved.' }
foreach ($file in @($Cli, $Worker, $workflow)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing input: $file" }
}
New-Item -ItemType Directory -Path $out | Out-Null
$sdkOut = Join-Path $out 'vendor-adapter-v2'
& robocopy $sdk $sdkOut /E /XJ /XD target node_modules .git .bgx-sdk __pycache__ /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "Public SDK copy failed: $LASTEXITCODE" }
New-Item -ItemType Directory -Path (Join-Path $sdkOut 'bin') | Out-Null
Copy-Item -LiteralPath $Cli -Destination (Join-Path $sdkOut 'bin/bgx-adapter-v2.exe')
Copy-Item -LiteralPath $Worker -Destination (Join-Path $sdkOut 'bin/bgx-adapter-v2-worker.exe')
$seed = Join-Path $out 'sample-source'
New-Item -ItemType Directory -Path $seed | Out-Null
Copy-Item -LiteralPath (Join-Path $source 'src'),(Join-Path $source 'schemas') -Destination $seed -Recurse
New-Item -ItemType Directory -Path (Join-Path $seed 'examples'),(Join-Path $seed 'workflow'),(Join-Path $seed 'protocol') | Out-Null
Copy-Item -LiteralPath $workflow -Destination (Join-Path $seed 'workflow/source.json')
Copy-Item -LiteralPath (Join-Path $sdk 'protocol/schema-policy.json') -Destination (Join-Path $seed 'protocol/schema-policy.json')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'resources/inspect.rs') -Destination (Join-Path $seed 'examples/inspect.rs')
$utf8 = [Text.UTF8Encoding]::new($false)
function Save([string]$Path, [string]$Text) { [IO.File]::WriteAllText($Path, $Text, $utf8) }
$oldId = 'local.bgx.wan22-animate'
$newId = 'local.example.comfy-wan22'
$capabilityPath = Join-Path $seed 'src/capability.rs'
Save $capabilityPath ([IO.File]::ReadAllText($capabilityPath).Replace($oldId, $newId))
$comfyPath = Join-Path $seed 'src/comfy.rs'
Save $comfyPath ([IO.File]::ReadAllText($comfyPath).Replace('../../../../contract/v2/protocol/schema-policy.json', '../protocol/schema-policy.json').Replace('../../../vendor-adapter-contract/v2/protocol/schema-policy.json', '../protocol/schema-policy.json'))
$manifest = Get-Content -LiteralPath (Join-Path $source 'manifest.json') -Raw | ConvertFrom-Json
$manifest.pluginId = $newId
$manifest.version = '1.0.0'
$manifest.displayName = 'ComfyUI Wan 2.2 author example'
$manifest.artifactSha256 = '0' * 64
Save (Join-Path $seed 'manifest.json') ($manifest | ConvertTo-Json -Depth 30)
$digest = (Get-FileHash -LiteralPath $workflow -Algorithm SHA256).Hash.ToLowerInvariant()
Save (Join-Path $seed 'workflow-templates.json') (@{schemaVersion=1;templates=@(@{sha256=$digest})} | ConvertTo-Json -Depth 5)
# The copied mapping constant is provenance only; runtime graph construction is unchanged.
$mappingPath = Join-Path $seed 'src/mapping.rs'
$oldDigest = (Get-Content -LiteralPath (Join-Path $source 'workflow-templates.json') -Raw | ConvertFrom-Json).templates[0].sha256
Save $mappingPath ([IO.File]::ReadAllText($mappingPath).Replace($oldDigest, $digest))
$cargo = [IO.File]::ReadAllText((Join-Path $sdkOut 'templates/rust/Cargo.toml'))
Save (Join-Path $seed 'Cargo.toml') ($cargo.Replace('bgx-adapter-template', 'bgx-comfy-wan22-example'))
foreach ($name in @('New-ComfyExample.ps1','Build-ComfyExample.ps1','Invoke-ComfyExample.ps1')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "tools/$name") -Destination $out
}
foreach ($name in @('README.zh-CN.md','AGENT-TASK.zh-CN.md','DEPENDENCIES.zh-CN.md','GENERATION-EXAMPLES.zh-CN.md')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $out
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'resources/request-move.json') -Destination (Join-Path $out 'request-move.json')
$head = (& git -C $repo rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot record the source revision.' }
$inputs = @($Cli, $Worker, $workflow, (Join-Path $sdk 'protocol/schema-policy.json')) + @(Get-ChildItem -LiteralPath (Join-Path $source 'src'),(Join-Path $source 'schemas') -File -Recurse | ForEach-Object FullName)
$files = @($inputs | ForEach-Object { @{file=[IO.Path]::GetRelativePath($repo,$_).Replace('\','/');sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant()} })
Save (Join-Path $out 'SOURCE.json') (@{schemaVersion=1;revision=$head;sourcePackage='packages/workflow-adapters/wan22-animate';examplePluginId=$newId;workflowSha256=$digest;files=$files;status='OFFLINE_CANDIDATE_LIVE_NOT_RUN'} | ConvertTo-Json -Depth 8)
Write-Output "Author kit: $out"
