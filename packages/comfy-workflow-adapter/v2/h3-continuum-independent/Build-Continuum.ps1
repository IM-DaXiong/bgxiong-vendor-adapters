[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Project,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
foreach ($path in @($Project,$OutputDirectory)) {
    if (-not [IO.Path]::IsPathFullyQualified($path)) { throw 'Use absolute project and output paths.' }
}
if (Test-Path -LiteralPath $OutputDirectory) { throw 'OutputDirectory must not exist.' }
$sdk = Join-Path $PSScriptRoot 'public-kit/vendor-adapter-v2'
$cli = Join-Path $sdk 'bin/bgx-adapter-v2.exe'
$worker = Join-Path $sdk 'bin/bgx-adapter-v2-worker.exe'
$buildArgs = @('build','--sdk',$sdk,'--project',$Project)
if ($Offline) { $buildArgs += '--offline' }
& $cli @buildArgs
if ($LASTEXITCODE -ne 0) { throw "SDK build failed: $LASTEXITCODE" }
& $cli check --sdk $sdk --worker $worker --project $Project
if ($LASTEXITCODE -ne 0) { throw "SDK check failed: $LASTEXITCODE" }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$runtime = Join-Path $OutputDirectory 'runtime'
& $cli pack --sdk $sdk --project $Project --out $runtime
if ($LASTEXITCODE -ne 0) { throw "SDK pack failed: $LASTEXITCODE" }
& $cli check --sdk $sdk --worker $worker --project $runtime
if ($LASTEXITCODE -ne 0) { throw "Packed runtime check failed: $LASTEXITCODE" }
$install = Join-Path $OutputDirectory 'install'
New-Item -ItemType Directory -Path $install | Out-Null
Get-ChildItem -LiteralPath $runtime -File | Copy-Item -Destination $install
Copy-Item -LiteralPath (Join-Path $Project 'workflow-templates.json') -Destination $install
Write-Output "Runtime: $runtime"
Write-Output "Application installation directory (verify installation, workflow association and a real run separately): $install"
