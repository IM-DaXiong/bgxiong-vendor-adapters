# A sample request builder over the public CLI, not an application task service.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('caps','probe','submit','query')][string]$Action,
    [Parameter(Mandatory)][string]$Runtime,
    [Parameter(Mandatory)][string]$Workspace,
    [Parameter(Mandatory)][string]$Endpoint,
    [string]$ProbeExecutable,
    [string]$RequestFile,
    [string]$SourceVideo,
    [string]$CharacterImage,
    [string]$SubmitRequestId
)
$ErrorActionPreference = 'Stop'
$sdk = Join-Path $PSScriptRoot 'vendor-adapter-v2'
$cli = Join-Path $sdk 'bin/bgx-adapter-v2.exe'
$worker = Join-Path $sdk 'bin/bgx-adapter-v2-worker.exe'
foreach ($path in @($Runtime,$Workspace)) {
    if (-not [IO.Path]::IsPathFullyQualified($path)) { throw 'Runtime and Workspace must be absolute.' }
}
$uri = [uri]$Endpoint
if (-not $uri.IsAbsoluteUri -or $uri.Scheme -notin @('http','https') -or $uri.Query -or $uri.Fragment -or $uri.UserInfo) {
    throw 'Endpoint must be an explicit HTTP(S) base URL without credentials, query or fragment.'
}
$configuration = @{endpointBaseUrl=$Endpoint.TrimEnd('/');credentialRef=$null}
$binding = 'comfy-author-example'
$media = @()
function Read-Json([string]$Path) { Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -Depth 100 }
function Save-Json([string]$Path, $Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 100), [Text.UTF8Encoding]::new($false))
}
function Media-Descriptor([string]$Id,[string]$Path,[string]$Kind) {
    $file = Get-Item -LiteralPath $Path
    $mime = @{'.mp4'='video/mp4';'.mov'='video/quicktime';'.png'='image/png';'.jpg'='image/jpeg';'.jpeg'='image/jpeg';'.webp'='image/webp'}[$file.Extension.ToLowerInvariant()]
    if (-not $mime) { throw "Unsupported sample media extension: $($file.Extension)" }
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    @{id=$Id;kind=$Kind;handle='pending';mime=$mime;fileName=($hash+$file.Extension.ToLowerInvariant());byteLength=([string]$file.Length)}
}
$payload = @{}
if ($Action -eq 'probe') { $payload = @{schemaVersion=2;kind='connectivity';configuration=$configuration} }
if ($Action -eq 'submit') {
    foreach ($path in @($RequestFile,$SourceVideo,$CharacterImage,$ProbeExecutable)) {
        if (-not $path -or -not [IO.Path]::IsPathFullyQualified($path) -or -not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw 'submit requires absolute, existing RequestFile, SourceVideo, CharacterImage and ProbeExecutable paths.'
        }
    }
    $catalog = Read-Json (Join-Path $Workspace 'capabilities.json')
    $contract = @($catalog.capabilityContracts | Where-Object capabilityKey -eq 'video.edit')
    if ($contract.Count -ne 1) { throw 'Expected the one video.edit contract of this sample.' }
    $operation = @($contract[0].operations | Where-Object id -eq 'local.example.comfy-wan22/move')
    if ($operation.Count -ne 1) { throw 'Sample Move operation is absent; run caps on the matching package.' }
    $media = @(@{id='source';path=$SourceVideo;kind='video'},@{id='replacement';path=$CharacterImage;kind='image'})
    $payload = @{
        schemaVersion=2;modelId=$contract[0].modelIds[0];capabilityKey=$contract[0].capabilityKey
        operationId=$operation[0].id;operationVersion=$operation[0].version;inputSchemaDigest=$operation[0].inputSchema.digest
        input=(Read-Json $RequestFile);configuration=$configuration;uploads=@()
        media=@((Media-Descriptor 'source' $SourceVideo 'video'),(Media-Descriptor 'replacement' $CharacterImage 'image'))
    }
}
if ($Action -eq 'query') {
    if (-not $SubmitRequestId) { throw 'query requires SubmitRequestId from the original accepted submit.' }
    $original = Read-Json (Join-Path $Workspace "inputs/$SubmitRequestId.json")
    $submitted = $original.payloadJson | ConvertFrom-Json -Depth 100
    $receipt = Read-Json (Join-Path $Workspace "receipts/$SubmitRequestId.json")
    if ($receipt.error) { throw 'Original submit failed; do not query or resubmit automatically.' }
    $accepted = $receipt.dataJson | ConvertFrom-Json -Depth 100
    if ($accepted.kind -ne 'accepted' -or -not $accepted.vendorTaskId) { throw 'Original submit was not accepted.' }
    $payload = @{
        schemaVersion=2;modelId=$submitted.modelId;capabilityKey=$submitted.capabilityKey
        operationId=$submitted.operationId;operationVersion=$submitted.operationVersion
        vendorTaskId=$accepted.vendorTaskId;resumeToken=$accepted.resumeToken;configuration=$configuration
    }
}
$probe = $null
if ($ProbeExecutable) {
    if (-not [IO.Path]::IsPathFullyQualified($ProbeExecutable) -or -not (Test-Path -LiteralPath $ProbeExecutable -PathType Leaf)) {
        throw 'ProbeExecutable must be an existing absolute ffprobe path.'
    }
    $probe = @{executable=$ProbeExecutable;timeoutMs=30000;frameTimestamps=$false}
}
$config = @{
    bindingId=$binding;bindingRevision=1;configurationRef='comfy-author-example-v1';configuration=$configuration
    trust=@{publishers=@();revokedPlugins=@();revokedVersions=@();revokedKeys=@();denylistedHosts=@();isFresh=$false;supportedFeatures=@()}
    network=@{origins=@($uri.GetLeftPart([UriPartial]::Authority));allowPrivate=$true;allowEmbeddedAuthorization=$false;deniedHosts=@();deniedNetworks=@()}
    credentials=@();media=$media;mediaTtlMs=3600000
    transport=@{handshakeTimeoutMs=10000;writeTimeoutMs=60000;shutdownTimeoutMs=10000;cancelGraceMs=100}
    probe=$probe
}
$create = -not (Test-Path -LiteralPath $Workspace)
# Let the CLI own workspace creation. Inputs/config live next to it until it exists.
$inputRoot = "$Workspace-inputs"
New-Item -ItemType Directory -Path $inputRoot -Force | Out-Null
$requestId = "comfy-$Action-$([guid]::NewGuid().ToString('N'))"
$configFile = Join-Path $inputRoot "$requestId-config.json"
$inputFile = Join-Path $inputRoot "$requestId.json"
$output = Join-Path $Workspace "receipts/$requestId.json"
Save-Json $configFile $config
$budget = if ($Action -in @('submit','query')) { 900000 } else { 60000 }
Save-Json $inputFile @{
    protocolVersion=2;operation=$(if($Action -eq 'caps'){'capabilities'}else{$Action})
    profileId=$binding;capabilitySlot=$(if($Action -eq 'caps'){'generic'}else{'video'});requestId=$requestId;budgetMs=$budget
    payloadJson=($payload | ConvertTo-Json -Depth 100 -Compress);bindingId=$binding;negotiatedVersion=2;enabledFeatures=@()
}
$cliArgs = @($Action,'--sdk',$sdk,'--worker',$worker,'--package',$Runtime,'--workspace',$Workspace,'--config',$configFile,'--input',$inputFile,'--output',$output)
if ($create) { $cliArgs += '--create-workspace' }
if ($Action -eq 'query') { $cliArgs += @('--request-id',$SubmitRequestId) }
& $cli @cliArgs
$exitCode = $LASTEXITCODE
if ($exitCode -ne 0) { throw "SDK $Action failed ($exitCode). Inspect $inputRoot and $Workspace/cli; do not resubmit automatically." }
$response = Read-Json $output
if ($response.error) { throw "SDK $Action returned $($response.error.code); inspect $output. No automatic retry." }
New-Item -ItemType Directory -Path (Join-Path $Workspace 'inputs') -Force | Out-Null
Copy-Item -LiteralPath $inputFile -Destination (Join-Path $Workspace "inputs/$requestId.json")
if ($Action -eq 'caps') {
    Save-Json (Join-Path $Workspace 'capabilities.json') ($response.dataJson | ConvertFrom-Json -Depth 100)
}
$appConfig = $config.Clone()
$appConfig.Remove('media')
$appConfig.budgetMs = @{capabilities=60000;probe=60000;submit=900000;query=900000;cancel=60000}
Save-Json (Join-Path $Workspace 'host-config.app.json') $appConfig
[pscustomobject]@{requestId=$requestId;receipt=$output;result=($response.dataJson | ConvertFrom-Json -Depth 100)} | ConvertTo-Json -Depth 20
