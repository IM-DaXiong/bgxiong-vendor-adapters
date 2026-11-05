param(
  [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path,
  [string]$SdkRoot = '',
  [string]$RuntimeRoot = 'E:\BreezeTTS2-assets\s1-staging\data\runtimes\tts\breeze\1.0.0',
  [string]$StoreRoot = 'E:\BreezeTTS2-assets\s1-staging\data\runtimes\tts\_store',
  [string]$ManagedPython = 'E:\BreezeTTS2-assets\s0-staging\python\python.exe',
  [string]$ProveRoot = 'E:\BreezeTTS2-assets\s4-staging',
  [string]$PackDir = '',
  [int]$Port = 18774,
  [string]$Text = 'Stream ok.',
  [switch]$SkipStream,
  [switch]$SkipBatch
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
if (-not $SdkRoot) { $SdkRoot = Join-Path $RepoRoot 'packages\vendor-adapter-contract\v2' }
if (-not $PackDir) { $PackDir = Join-Path $RepoRoot 'packages\tts-adapters\breeze\dist-pack-c' }
$Cli = Join-Path $SdkRoot 'target\debug\bgx-adapter-v2.exe'
$Worker = Join-Path $SdkRoot 'target\debug\bgx-adapter-v2-worker.exe'
if (-not (Test-Path (Join-Path $PackDir 'manifest.json'))) { throw "C pack missing: $PackDir" }
New-Item -ItemType Directory -Force -Path $ProveRoot | Out-Null
$LogDir = Join-Path $ProveRoot 'logs'
$OutDir = Join-Path $ProveRoot 'out'
New-Item -ItemType Directory -Force -Path $LogDir,$OutDir | Out-Null
function Write-Json($Path, $Obj) {
  $json = $Obj | ConvertTo-Json -Depth 30 -Compress
  [System.IO.File]::WriteAllText($Path, $json, [Text.UTF8Encoding]::new($false))
}
function Wait-Health {
  $deadline = (Get-Date).AddMinutes(15)
  while ((Get-Date) -lt $deadline) {
    try {
      $r = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/health" -TimeoutSec 3
      if ($r) { return $r }
    } catch { Start-Sleep -Seconds 2 }
  }
  throw 'health timeout'
}
function New-Config($Origin) {
  return @{
    bindingId = 'binding'
    bindingRevision = 1
    configurationRef = 'breeze-s4-c'
    configuration = @{ schemaVersion = 1; origin = $Origin; instanceId = 'breeze-s4-c'; credentialSlot = 'none' }
    trust = @{ publishers = @(); revokedPlugins = @(); revokedVersions = @(); revokedKeys = @(); denylistedHosts = @(); isFresh = $false; supportedFeatures = @('stream-events-v1') }
    network = @{ origins = @($Origin); allowPrivate = $true; allowEmbeddedAuthorization = $false; deniedHosts = @(); deniedNetworks = @() }
    credentials = @(); media = @(); mediaTtlMs = 600000
    transport = @{ handshakeTimeoutMs = 5000; writeTimeoutMs = 5000; shutdownTimeoutMs = 5000; cancelGraceMs = 100 }
    probe = $null
  }
}
function Invoke-Submit($Ws, $ConfigPath, $InputPath, $OutPath) {
  if (Test-Path $Ws) { Remove-Item -Recurse -Force $Ws }
  $stderrFile = Join-Path $LogDir ("submit-" + [IO.Path]::GetFileNameWithoutExtension($InputPath) + ".stderr.log")
  $prev = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  $stdout = & $Cli submit --sdk $SdkRoot --worker $Worker --package $PackDir --workspace $Ws --config $ConfigPath --input $InputPath --output $OutPath --create-workspace 2> $stderrFile
  $code = $LASTEXITCODE
  $ErrorActionPreference = $prev
  [System.IO.File]::WriteAllText((Join-Path $LogDir ("submit-" + [IO.Path]::GetFileNameWithoutExtension($InputPath) + ".stdout.log")), ($stdout | Out-String), [Text.UTF8Encoding]::new($false))
  if ($code -ne 0) {
    if (Test-Path $stderrFile) { Get-Content -Raw -Encoding UTF8 $stderrFile | Write-Host }
    throw "SDK submit failed exit=$code for $InputPath"
  }
  return (Get-Content -Raw -Encoding UTF8 $OutPath | ConvertFrom-Json)
}

$env:BGXIONG_MANAGED_PYTHON = $ManagedPython
$env:BGX_TTS_STORE_ROOT = $StoreRoot
$env:BGX_EXTERNAL_PROVISIONED = '1'
$env:HF_HUB_OFFLINE = '1'
$env:TRANSFORMERS_OFFLINE = '1'
$env:BGX_TTS_SIDECAR_PORT = "$Port"
$sox = 'E:\BreezeTTS2-assets\s0-staging\sox\sox-14.4.2'
if (Test-Path (Join-Path $sox 'sox.exe')) { $env:PATH = "$sox;$env:PATH" }
try {
  $conn = Get-NetTCPConnection -LocalPort $Port -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($conn -and $conn.OwningProcess) { Stop-Process -Id $conn.OwningProcess -Force -ErrorAction SilentlyContinue; Start-Sleep -Seconds 1 }
} catch {}
$launcher = Join-Path $RuntimeRoot 'start-sidecar.ps1'
if (-not (Test-Path $launcher)) { throw "launcher missing: $launcher" }
Write-Host ('Starting companion port=' + $Port); Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File', $launcher, '--port', ([string]$Port)) -WindowStyle Hidden | Out-Null
$health = Wait-Health
Write-Json (Join-Path $LogDir 'health.json') $health
Write-Host 'HEALTH OK'
$origin = "http://127.0.0.1:$Port"
$configPath = Join-Path $ProveRoot 'config-c.json'
Write-Json $configPath (New-Config $origin)

$synthDigest = 'f51eacb5d1d76023638e8c66646132abca32a5be4599d115f2c5102b822360c2'
$batchDigest = '22277bf9604cb68141e8c7abc32255d806597da9fb89c5306fef158bed4e7fb6'
$results = @{}

if (-not $SkipBatch) {
  Write-Host '=== T20 batch ==='
  $ws = Join-Path $ProveRoot 'workspace-c-batch'
  $payload = @{
    schemaVersion = 2
    modelId = 'BreezeBlue/Breeze-TTS-2'
    capabilityKey = 'audio.synthesize'
    operationId = 'std/speech.batch'
    operationVersion = 1
    inputSchemaDigest = $batchDigest
    input = @{
      schemaVersion = 1
      items = @(
        @{ itemId = 'i1'; input = @{ schemaVersion = 1; text = 'Batch one.' } },
        @{ itemId = 'i2'; input = @{ schemaVersion = 1; text = 'Batch two.' } }
      )
    }
    media = @(); uploads = @()
    configuration = @{ schemaVersion = 1; origin = $origin; instanceId = 'breeze-s4-c'; credentialSlot = 'none' }
  }
  $submit = @{
    protocolVersion = 2; operation = 'submit'; profileId = 'binding'; bindingId = 'binding'
    capabilitySlot = 'speech'; requestId = 's4-c-batch-1'; budgetMs = 600000; negotiatedVersion = 2
    enabledFeatures = @('stream-events-v1')
    payloadJson = ($payload | ConvertTo-Json -Depth 30 -Compress)
  }
  $inputPath = Join-Path $ProveRoot 'submit-c-batch.input.json'
  $outPath = Join-Path $ws 'receipts\result.json'
  Write-Json $inputPath $submit
  $result = Invoke-Submit $ws $configPath $inputPath $outPath
  $data = $result.dataJson | ConvertFrom-Json
  if ($data.kind -ne 'completed') { throw "batch kind=$($data.kind)" }
  $n = @($data.outputs).Count
  if ($n -lt 2) { throw "batch outputs=$n expected>=2" }
  $i = 0
  foreach ($o in $data.outputs) {
    $i++
    $name = $o.media.fileName
    $src = Join-Path $ws "objects\$name"
    $dst = Join-Path $OutDir ("s4_c_batch_$i.wav")
    Copy-Item -Force $src $dst
  }
  Write-Host "S4_C_BATCH_PASS outputs=$n"
  $results.batch = 'PASS'
}

if (-not $SkipStream) {
  Write-Host '=== T19 stream ==='
  $ws = Join-Path $ProveRoot 'workspace-c-stream'
  $payload = @{
    schemaVersion = 2
    modelId = 'BreezeBlue/Breeze-TTS-2'
    capabilityKey = 'audio.synthesize'
    operationId = 'std/speech.synthesize'
    operationVersion = 1
    inputSchemaDigest = $synthDigest
    input = @{ schemaVersion = 1; text = $Text; execution = @{ delivery = 'stream' } }
    media = @(); uploads = @()
    configuration = @{ schemaVersion = 1; origin = $origin; instanceId = 'breeze-s4-c'; credentialSlot = 'none' }
  }
  $submit = @{
    protocolVersion = 2; operation = 'submit'; profileId = 'binding'; bindingId = 'binding'
    capabilitySlot = 'speech'; requestId = 's4-c-stream-1'; budgetMs = 600000; negotiatedVersion = 2
    enabledFeatures = @('stream-events-v1')
    payloadJson = ($payload | ConvertTo-Json -Depth 30 -Compress)
  }
  $inputPath = Join-Path $ProveRoot 'submit-c-stream.input.json'
  $outPath = Join-Path $ws 'receipts\result.json'
  Write-Json $inputPath $submit
  $result = Invoke-Submit $ws $configPath $inputPath $outPath
  $data = $result.dataJson | ConvertFrom-Json
  if ($data.kind -ne 'completed') { throw "stream kind=$($data.kind)" }
  $sha = [System.BitConverter]::ToString([System.Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes('s4-c-stream-1'))).Replace('-','').ToLower()
  $eventsPath = Join-Path $ws "cli\$sha\events.jsonl"
  if (-not (Test-Path $eventsPath)) { throw "events.jsonl missing: $eventsPath" }
  $elines = @(Get-Content -Encoding UTF8 $eventsPath | Where-Object { $_.Trim().Length -gt 0 })
  if ($elines.Count -lt 1) { throw 'events.jsonl empty' }
  $sample = @($elines | Select-Object -First 5)
  [System.IO.File]::WriteAllText((Join-Path $LogDir 'stream-events-sample.jsonl'), (($sample -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
  $outs = @($data.outputs)
  if ($outs.Count -lt 1) { throw 'stream missing outputs' }
  $name = $outs[0].media.fileName
  Copy-Item -Force (Join-Path $ws "objects\$name") (Join-Path $OutDir 's4_c_stream.wav')
  Write-Host "S4_C_STREAM_PASS events=$($elines.Count)"
  $results.stream = 'PASS'
}

Write-Json (Join-Path $LogDir 's4-results.json') $results
Write-Host 'S4_C_SDK_PASS'
