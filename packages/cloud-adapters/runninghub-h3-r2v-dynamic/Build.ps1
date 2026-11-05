param(
    [Parameter(Mandatory=$true)][string]$Sdk,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [switch]$Online
)
$ErrorActionPreference = 'Stop'
$Sdk = (Resolve-Path -LiteralPath $Sdk).Path
$Project = $PSScriptRoot
if (Test-Path -LiteralPath $OutputDirectory) { throw 'OutputDirectory must not exist.' }
$TemplatePath = Join-Path $Project 'installation/adapter-install.template.json'
$Installation = Get-Content -LiteralPath $TemplatePath -Raw | ConvertFrom-Json
if ($Installation.schemaVersion -ne 1 -or $Installation.packageDirectory -ne 'package') { throw 'Invalid installation layout.' }
if ($Installation.hostConfig.credentials.Count -ne 0 -or $Installation.authentication.source -ne 'userInput') { throw 'Installation must use the application credential broker.' }
$Origins = @($Installation.hostConfig.network.origins)
foreach ($Field in $Installation.authentication.fields) {
    foreach ($Origin in $Field.origins) {
        if ($Origin -notin $Origins) { throw 'Credential origin is outside the declared network origins.' }
    }
}
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
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$PackageDirectory = Join-Path $OutputDirectory 'package'
& $Cli pack --sdk $Sdk --project $Project --out $PackageDirectory
if ($LASTEXITCODE -ne 0) { throw 'Standard SDK pack failed.' }
& $Cli check --sdk $Sdk --worker $Worker --project $PackageDirectory
if ($LASTEXITCODE -ne 0) { throw 'Runtime package validation failed.' }
$Installation.manifestSha256 = (Get-FileHash -LiteralPath (Join-Path $PackageDirectory 'manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant()
$Installation | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'adapter-install.json') -Encoding utf8NoBOM
$Saved = Get-Content -LiteralPath (Join-Path $OutputDirectory 'adapter-install.json') -Raw | ConvertFrom-Json
if ($Saved.manifestSha256 -ne (Get-FileHash -LiteralPath (Join-Path $PackageDirectory 'manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant()) { throw 'Installation manifest digest mismatch.' }
Write-Output "Installable package: $OutputDirectory"
