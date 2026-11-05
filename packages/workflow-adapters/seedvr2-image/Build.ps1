[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Sdk,
    [Parameter(Mandatory)][string]$RuntimeSdk,
    [Parameter(Mandatory)][string]$Cli,
    [Parameter(Mandatory)][string]$Worker,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
foreach ($path in @($Sdk,$RuntimeSdk,$Cli,$Worker,$OutputDirectory)) {
    if (-not [IO.Path]::IsPathFullyQualified($path)) { throw 'All paths must be absolute.' }
}
if (Test-Path -LiteralPath $OutputDirectory) { throw 'OutputDirectory already exists; preserve accepted artifacts.' }
$buildArgs = @('build','--sdk',$Sdk,'--project',$PSScriptRoot)
if ($Offline) { $buildArgs += '--offline' }
& $Cli @buildArgs
if ($LASTEXITCODE -ne 0) { throw "SDK author build failed: $LASTEXITCODE" }
& $Cli check --sdk $RuntimeSdk --worker $Worker --project $PSScriptRoot
if ($LASTEXITCODE -ne 0) { throw "SDK package check failed: $LASTEXITCODE" }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$runtime = Join-Path $OutputDirectory 'runtime'
& $Cli pack --sdk $Sdk --project $PSScriptRoot --out $runtime
if ($LASTEXITCODE -ne 0) { throw "SDK pack failed: $LASTEXITCODE" }
& $Cli check --sdk $RuntimeSdk --worker $Worker --project $runtime
if ($LASTEXITCODE -ne 0) { throw "Packed runtime check failed: $LASTEXITCODE" }
$install = Join-Path $OutputDirectory 'install'
New-Item -ItemType Directory -Path $install | Out-Null
Get-ChildItem -LiteralPath $runtime -File | Copy-Item -Destination $install
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'workflow-templates.json') -Destination $install
Write-Output "Install directory: $install"
