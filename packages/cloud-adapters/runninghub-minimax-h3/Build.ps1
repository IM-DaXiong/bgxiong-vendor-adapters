param(
    [Parameter(Mandatory=$true)][string]$Sdk,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [switch]$Online
)
$ErrorActionPreference = 'Stop'
$Sdk = (Resolve-Path -LiteralPath $Sdk).Path
$Project = $PSScriptRoot
if (-not [IO.Path]::IsPathFullyQualified($OutputDirectory)) { throw 'OutputDirectory must be absolute.' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'OutputDirectory must not exist.' }
$RuntimeDirectory = Join-Path $OutputDirectory 'package'
$Cli = Join-Path $Sdk 'bin/bgx-adapter-v2.exe'
$Worker = Join-Path $Sdk 'bin/bgx-adapter-v2-worker.exe'
foreach ($tool in @($Cli, $Worker)) {
    if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) { throw "Required SDK tool missing: $tool" }
}
$BuildArguments = @('build', '--sdk', $Sdk, '--project', $Project)
if (-not $Online) { $BuildArguments += '--offline' }
& $Cli @BuildArguments
if ($LASTEXITCODE -ne 0) { throw 'Standard SDK build failed.' }
& $Cli check --sdk $Sdk --worker $Worker --project $Project
if ($LASTEXITCODE -ne 0) { throw 'Source package validation failed.' }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
& $Cli pack --sdk $Sdk --project $Project --out $RuntimeDirectory
if ($LASTEXITCODE -ne 0) { throw 'Standard SDK pack failed.' }
& $Cli check --sdk $Sdk --worker $Worker --project $RuntimeDirectory
if ($LASTEXITCODE -ne 0) { throw 'Runtime package validation failed.' }
$Envelope = Get-Content -LiteralPath (Join-Path $Project 'installation-template.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$Envelope.manifestSha256 = (Get-FileHash -LiteralPath (Join-Path $RuntimeDirectory 'manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'adapter-install.json'), ($Envelope | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
