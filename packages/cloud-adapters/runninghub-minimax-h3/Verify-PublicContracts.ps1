param(
    [Parameter(Mandatory=$true)][string]$Sdk,
    [Parameter(Mandatory=$true)][string]$InstallationDirectory,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$Sdk = (Resolve-Path -LiteralPath $Sdk).Path.Replace('\','/')
$InstallationDirectory = (Resolve-Path -LiteralPath $InstallationDirectory).Path
if (-not [IO.Path]::IsPathFullyQualified($OutputDirectory) -or (Test-Path -LiteralPath $OutputDirectory)) {
    throw 'OutputDirectory must be a new absolute directory.'
}
New-Item -ItemType Directory -Path (Join-Path $OutputDirectory 'src') -Force | Out-Null
$Fixture = Join-Path $OutputDirectory 'actual-contract-fixtures.json'
$PreviousFixture = $env:RUNNINGHUB_CONTRACT_FIXTURES
$PreviousTarget = $env:CARGO_TARGET_DIR
try {
    $env:RUNNINGHUB_CONTRACT_FIXTURES = $Fixture
    $env:CARGO_TARGET_DIR = Join-Path $OutputDirectory 'target-native'
    & cargo test --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'Native adapter regression failed.' }
    $Manifest = @"
[workspace]
[package]
name="runninghub-public-contract-verifier"
version="0.1.0"
edition="2021"
[dependencies]
bgx-adapter-v2-sdk={path="$Sdk/sdk/rust"}
bgx-adapter-v2-validator={path="$Sdk/crates/validator"}
bgx-adapter-v2-reference-host={path="$Sdk/crates/reference-host"}
serde_json="=1.0.149"
"@
    [IO.File]::WriteAllText((Join-Path $OutputDirectory 'Cargo.toml'), $Manifest, [Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'verification/public-contracts.rs') -Destination (Join-Path $OutputDirectory 'src/main.rs')
    $env:CARGO_TARGET_DIR = Join-Path $OutputDirectory 'target-verifier'
    & cargo run --manifest-path (Join-Path $OutputDirectory 'Cargo.toml') -- $Fixture (Join-Path $InstallationDirectory 'adapter-install.json')
    if ($LASTEXITCODE -ne 0) { throw 'Public SDK contract or network-policy verification failed.' }
} finally {
    $env:RUNNINGHUB_CONTRACT_FIXTURES = $PreviousFixture
    $env:CARGO_TARGET_DIR = $PreviousTarget
}
