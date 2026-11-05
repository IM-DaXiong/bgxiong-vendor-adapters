[CmdletBinding()]
param([Parameter(Mandatory)][string]$Project)
$ErrorActionPreference = 'Stop'
if (-not [IO.Path]::IsPathFullyQualified($Project)) { throw 'Project must be absolute.' }
if (Test-Path -LiteralPath $Project) { throw 'Project must not exist.' }
$sdk = Join-Path $PSScriptRoot 'vendor-adapter-v2'
$cli = Join-Path $sdk 'bin/bgx-adapter-v2.exe'
& $cli init --sdk $sdk --dir $Project
if ($LASTEXITCODE -ne 0) { throw "SDK init failed: $LASTEXITCODE" }
$seed = Join-Path $PSScriptRoot 'sample-source'
Get-ChildItem -LiteralPath $seed -Force | Copy-Item -Destination $Project -Recurse -Force
$lock = Join-Path $Project 'Cargo.lock'
$text = [IO.File]::ReadAllText($lock).Replace('name = "bgx-adapter-template"', 'name = "bgx-comfy-wan22-example"')
[IO.File]::WriteAllText($lock, $text, [Text.UTF8Encoding]::new($false))
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'README.zh-CN.md') -Destination (Join-Path $Project 'README.md') -Force
Write-Output "Independent project: $Project"
