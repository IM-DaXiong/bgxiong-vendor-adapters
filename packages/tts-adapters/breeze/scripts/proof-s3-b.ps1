param(
  [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path,
  [string]$SdkRoot = '',
  [string]$RuntimeRoot = 'E:\BreezeTTS2-assets\s1-staging\data\runtimes\tts\breeze\1.0.0',
  [string]$StoreRoot = 'E:\BreezeTTS2-assets\s1-staging\data\runtimes\tts\_store',
  [string]$ManagedPython = 'E:\BreezeTTS2-assets\s0-staging\python\python.exe',
  [string]$ProveRoot = 'E:\BreezeTTS2-assets\s3-staging',
  [string]$PackDir = '',
  [int]$Port = 18772,
  [string]$Text = '',
  [string]$Seed = '7'
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
if (-not $Text) { $Text = [string]([char]0x4F60)+[string]([char]0x597D)+[string]([char]0x3002) }
if (-not $SdkRoot) { $SdkRoot = Join-Path $RepoRoot 'packages\vendor-adapter-contract\v2' }
if (-not $PackDir) { $PackDir = Join-Path $RepoRoot 'packages\tts-adapters\breeze\dist-pack-b' }
$Cli = Join-Path $SdkRoot 'target\debug\bgx-adapter-v2.exe'
$Worker = Join-Path $SdkRoot 'target\debug\bgx-adapter-v2-worker.exe'
if (-not (Test-Path (Join-Path $PackDir 'manifest.json'))) { throw "B pack missing: $PackDir" }
New-Item -ItemType Directory -Force -Path $ProveRoot | Out-Null
$LogDir = Join-Path $ProveRoot 'logs'
$OutDir = Join-Path $ProveRoot 'out'
New-Item -ItemType Directory -Force -Path $LogDir,$OutDir | Out-Null
function Write-Json($Path, $Obj) {
  $json = $Obj | ConvertTo-Json -Depth 20 -Compress
  [System.IO.File]::WriteAllText($Path, $json, [Text.UTF8Encoding]::new($false))
}
$env:BGXIONG_MANAGED_PYTHON = $ManagedPython
$env:BGX_TTS_STORE_ROOT = $StoreRoot
$env:BGX_EXTERNAL_PROVISIONED = '1'
$env:HF_HUB_OFFLINE = '1'
$env:TRANSFORMERS_OFFLINE = '1'
$sox = 'E:\BreezeTTS2-assets\s0-staging\sox\sox-14.4.2'
if (Test-Path (Join-Path $sox 'sox.exe')) { $env:PATH = "$sox;$env:PATH" }
try {
  $conn = Get-NetTCPConnection -LocalPort $Port -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($conn -and $conn.OwningProcess) {
    Stop-Process -Id $conn.OwningProcess -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
  }
} catch {}
$launcher = Join-Path $RuntimeRoot 'start-sidecar.ps1'
$stderrLog = Join-Path $LogDir 'sidecar.stderr.log'
$stdoutLog = Join-Path $LogDir 'sidecar.stdout.log'
Write-Host "Starting companion port=$Port root=$RuntimeRoot"
$proc = Start-Process -FilePath 'powershell.exe' -ArgumentList @(
  '-NoProfile','-ExecutionPolicy','Bypass','-File',$launcher,'--port',"$Port"
) -PassThru -WindowStyle Hidden -RedirectStandardError $stderrLog -RedirectStandardOutput $stdoutLog
function Wait-Health([int]$TimeoutSec = 900) {
  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  while ((Get-Date) -lt $deadline) {
    if ($proc.HasExited) { throw "companion exited early code=$($proc.ExitCode). See $stderrLog" }
    try {
      $r = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/health" -TimeoutSec 5
      if ($r.status -eq 'ok' -and $r.modelLoaded -eq $true) { return $r }
      Write-Host ("health status={0} loaded={1}" -f $r.status, $r.modelLoaded)
    } catch {
      Write-Host ("waiting health: {0}" -f $_.Exception.Message)
    }
    Start-Sleep -Seconds 5
  }
  throw 'health timeout'
}
try {
  $health = Wait-Health
  Write-Json (Join-Path $LogDir 'health.json') $health
  Write-Host 'HEALTH OK'
  $origin = "http://127.0.0.1:$Port"
  $ws = Join-Path $ProveRoot 'workspace-b-submit'
  if (Test-Path $ws) { Remove-Item -Recurse -Force $ws }
  # workspace must be empty for --create-workspace (CLI creates receipts)
  $config = @{
    bindingId = 'binding'
    bindingRevision = 1
    configurationRef = 'breeze-s3-b'
    configuration = @{
      schemaVersion = 1
      origin = $origin
      instanceId = 'breeze-s3-b'
      credentialSlot = 'none'
    }
    trust = @{
      publishers = @()
      revokedPlugins = @()
      revokedVersions = @()
      revokedKeys = @()
      denylistedHosts = @()
      isFresh = $false
      supportedFeatures = @()
    }
    network = @{
      origins = @($origin)
      allowPrivate = $true
      allowEmbeddedAuthorization = $false
      deniedHosts = @()
      deniedNetworks = @()
    }
    credentials = @()
    media = @()
    mediaTtlMs = 600000
    transport = @{
      handshakeTimeoutMs = 5000
      writeTimeoutMs = 5000
      shutdownTimeoutMs = 5000
      cancelGraceMs = 100
    }
    probe = $null
  }
  $configPath = Join-Path $ProveRoot 'config-b.json'
  Write-Json $configPath $config
  $inputDigest = 'f51eacb5d1d76023638e8c66646132abca32a5be4599d115f2c5102b822360c2'
  $payload = @{
    schemaVersion = 2
    modelId = 'BreezeBlue/Breeze-TTS-2'
    capabilityKey = 'audio.synthesize'
    operationId = 'std/speech.synthesize'
    operationVersion = 1
    inputSchemaDigest = $inputDigest
    input = @{ schemaVersion = 1; text = $Text; seed = $Seed }
    media = @()
    uploads = @()
    configuration = @{
      schemaVersion = 1
      origin = $origin
      instanceId = 'breeze-s3-b'
      credentialSlot = 'none'
    }
  }
  $submitInput = @{
    protocolVersion = 2
    operation = 'submit'
    profileId = 'binding'
    bindingId = 'binding'
    capabilitySlot = 'speech'
    requestId = 's3-b-seed-1'
    budgetMs = 600000
    negotiatedVersion = 2
    enabledFeatures = @()
    payloadJson = ($payload | ConvertTo-Json -Depth 20 -Compress)
  }
  $inputPath = Join-Path $ProveRoot 'submit-b.input.json'
  $outPath = Join-Path $ws 'receipts\result.json'
  Write-Json $inputPath $submitInput
  Write-Host "SDK submit B pack seed=$Seed ..."
  $stderrFile = Join-Path $LogDir 'submit-b.stderr.log'
  # Match S1: call with redirect; temporarily soften preference so JSON stderr does not abort
  $prev = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  $stdout = & $Cli submit --sdk $SdkRoot --worker $Worker --package $PackDir --workspace $ws --config $configPath --input $inputPath --output $outPath --create-workspace 2> $stderrFile
  $code = $LASTEXITCODE
  $ErrorActionPreference = $prev
  [System.IO.File]::WriteAllText((Join-Path $LogDir 'submit-b.stdout.log'), ($stdout | Out-String), [Text.UTF8Encoding]::new($false))
  Write-Host "submit exit=$code"
  if ($code -ne 0) {
    if (Test-Path $stderrFile) { Get-Content -Raw -Encoding UTF8 $stderrFile | Write-Host }
    throw "SDK submit B failed exit=$code"
  }
  $result = Get-Content -Raw -Encoding UTF8 $outPath | ConvertFrom-Json
  $data = $result.dataJson | ConvertFrom-Json
  Write-Json (Join-Path $LogDir 'submit-b.data.json') $data
  if ($data.kind -ne 'completed') { throw "expected completed got $($data.kind)" }
  $ap = $data.appliedParams
  if (-not $ap -or [string]$ap.seed -ne $Seed) {
    throw "appliedParams.seed expected $Seed got $($ap | ConvertTo-Json -Compress)"
  }
  Write-Host "appliedParams.seed OK=$Seed"
  $retained = Get-ChildItem -Path $ws -Recurse -File -Filter *.wav -ErrorAction SilentlyContinue
  $sample = Join-Path $OutDir 's3_b_sdk_seed.wav'
  if ($retained) {
    Copy-Item -Force $retained[0].FullName $sample
    Write-Host "SAMPLE $($retained[0].FullName) -> $sample bytes=$($retained[0].Length)"
  } else {
    Write-Host 'WARN: no wav found in workspace'
  }
  $aMan = Get-Content -Raw -Encoding UTF8 (Join-Path $RepoRoot 'packages\tts-adapters\breeze\dist-pack\manifest.json') | ConvertFrom-Json
  $bMan = Get-Content -Raw -Encoding UTF8 (Join-Path $PackDir 'manifest.json') | ConvertFrom-Json
  if ($aMan.version -eq $bMan.version) { throw 'A/B versions must differ' }
  if ($aMan.artifactSha256 -eq $bMan.artifactSha256) { throw 'A/B artifactSha256 must differ' }
  Write-Host "A=$($aMan.version) B=$($bMan.version) Asha=$($aMan.artifactSha256.Substring(0,12)) Bsha=$($bMan.artifactSha256.Substring(0,12))"
  Write-Host 'S3_B_SDK_SEED_PASS'
}
finally {
  if ($proc -and -not $proc.HasExited) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
  }
  try {
    $conn = Get-NetTCPConnection -LocalPort $Port -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($conn -and $conn.OwningProcess) { Stop-Process -Id $conn.OwningProcess -Force -ErrorAction SilentlyContinue }
  } catch {}
}
