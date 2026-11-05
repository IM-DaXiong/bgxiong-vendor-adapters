# Thin public CLI client: one submit, later query only its original accepted request.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('caps','probe','submit','query')][string]$Action,
    [Parameter(Mandatory)][string]$Runtime,
    [Parameter(Mandatory)][string]$Workspace,
    [Parameter(Mandatory)][string]$Endpoint,
    [string]$RequestFile,
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
    throw 'Endpoint must be an HTTP(S) base URL without credentials, query or fragment.'
}
function Read-Json([string]$Path) { Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -Depth 100 }
function Save-Json([string]$Path,$Value) {
    [IO.File]::WriteAllText($Path,($Value | ConvertTo-Json -Depth 100),[Text.UTF8Encoding]::new($false))
}
$binding = 'comfy-qwen-author-example'
$configuration = @{endpointBaseUrl=$Endpoint.TrimEnd('/')}
$payload = @{}
if ($Action -eq 'probe') { $payload = @{schemaVersion=2;kind='connectivity';configuration=$configuration} }
if ($Action -eq 'submit') {
    if (-not $RequestFile) { throw 'submit requires RequestFile.' }
    $caps = Read-Json (Join-Path $Workspace 'capabilities.json')
    $contracts = @($caps.capabilityContracts | Where-Object capabilityKey -eq 'image.generate')
    if ($contracts.Count -ne 1 -or @($contracts[0].operations).Count -ne 1 -or @($contracts[0].modelIds).Count -ne 1) {
        throw 'Expected one image operation/model; run caps against the same package first.'
    }
    $contract=$contracts[0]; $operation=$contract.operations[0]
    $payload=@{schemaVersion=2;modelId=$contract.modelIds[0];capabilityKey=$contract.capabilityKey
        operationId=$operation.id;operationVersion=$operation.version;inputSchemaDigest=$operation.inputSchema.digest
        input=(Read-Json $RequestFile);configuration=$configuration;media=@();uploads=@()}
}
if ($Action -eq 'query') {
    if (-not $SubmitRequestId) { throw 'query requires the original SubmitRequestId.' }
    $request=Read-Json (Join-Path $Workspace "inputs/$SubmitRequestId.json")
    $original=$request.payloadJson | ConvertFrom-Json -Depth 100
    $receipt=Read-Json (Join-Path $Workspace "receipts/$SubmitRequestId.json")
    if ($receipt.error) { throw 'Original submit failed; do not resubmit automatically.' }
    $accepted=$receipt.dataJson | ConvertFrom-Json -Depth 100
    if ($accepted.kind -ne 'accepted' -or -not $accepted.vendorTaskId) { throw 'Original submit was not accepted.' }
    $payload=@{schemaVersion=2;modelId=$original.modelId;capabilityKey=$original.capabilityKey
        operationId=$original.operationId;operationVersion=$original.operationVersion;configuration=$configuration
        vendorTaskId=$accepted.vendorTaskId;resumeToken=$accepted.resumeToken}
}
$config=@{bindingId=$binding;bindingRevision=1;configurationRef='comfy-qwen-author-v1';configuration=$configuration
    trust=@{publishers=@();revokedPlugins=@();revokedVersions=@();revokedKeys=@();denylistedHosts=@();isFresh=$false;supportedFeatures=@()}
    network=@{origins=@($uri.GetLeftPart([UriPartial]::Authority));allowPrivate=$true;allowEmbeddedAuthorization=$false;deniedHosts=@();deniedNetworks=@()}
    credentials=@();media=@();mediaTtlMs=3600000
    transport=@{handshakeTimeoutMs=10000;writeTimeoutMs=60000;shutdownTimeoutMs=10000;cancelGraceMs=100}}
$create=-not (Test-Path -LiteralPath $Workspace)
$inputRoot="$Workspace-inputs"
New-Item -ItemType Directory -Path $inputRoot -Force | Out-Null
$requestId="qwen-$Action-$([guid]::NewGuid().ToString('N'))"
$configFile=Join-Path $inputRoot "$requestId-config.json"
$inputFile=Join-Path $inputRoot "$requestId.json"
$output=Join-Path $Workspace "receipts/$requestId.json"
Save-Json $configFile $config
Save-Json $inputFile @{protocolVersion=2;operation=$(if($Action -eq 'caps'){'capabilities'}else{$Action})
    profileId=$binding;capabilitySlot=$(if($Action -eq 'caps'){'generic'}else{'image'});requestId=$requestId;budgetMs=120000
    payloadJson=($payload | ConvertTo-Json -Depth 100 -Compress);bindingId=$binding;negotiatedVersion=2;enabledFeatures=@()}
$cliArgs=@($Action,'--sdk',$sdk,'--worker',$worker,'--package',$Runtime,'--workspace',$Workspace,'--config',$configFile,'--input',$inputFile,'--output',$output)
if ($create) { $cliArgs+='--create-workspace' }
if ($Action -eq 'query') { $cliArgs+=@('--request-id',$SubmitRequestId) }
& $cli @cliArgs
if ($LASTEXITCODE -ne 0) { throw "SDK $Action failed; inspect $inputRoot and $Workspace/cli. No automatic retry." }
$response=Read-Json $output
if ($response.error) { throw "SDK $Action returned $($response.error.code); inspect $output. No automatic retry." }
New-Item -ItemType Directory -Path (Join-Path $Workspace 'inputs') -Force | Out-Null
Copy-Item -LiteralPath $inputFile -Destination (Join-Path $Workspace "inputs/$requestId.json")
if ($Action -eq 'caps') { Save-Json (Join-Path $Workspace 'capabilities.json') ($response.dataJson | ConvertFrom-Json -Depth 100) }
$appConfig=$config.Clone(); $appConfig.Remove('media')
$appConfig.budgetMs=@{capabilities=60000;probe=60000;submit=120000;query=120000;cancel=60000}
Save-Json (Join-Path $Workspace 'host-config.app.json') $appConfig
[pscustomobject]@{requestId=$requestId;receipt=$output;result=($response.dataJson | ConvertFrom-Json -Depth 100)} | ConvertTo-Json -Depth 20
