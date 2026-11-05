# VoxCPM2 Sidecar launcher (Windows SSOT)
# ASCII-ONLY on purpose: Windows PowerShell 5.1 GBK parse hazard.
param([int]$Port = 0)

$ErrorActionPreference = 'Stop'
$RuntimeRoot = $PSScriptRoot
$VenvDir = Join-Path $RuntimeRoot '.venv'
$BootstrapJsonl = Join-Path $RuntimeRoot 'sidecar.bootstrap.jsonl'
$StderrLog = Join-Path $RuntimeRoot 'sidecar.stderr.log'
$PidFile = Join-Path $RuntimeRoot 'sidecar.pid'
$Requirements = Join-Path $RuntimeRoot 'requirements.txt'
$ServerPy = Join-Path $RuntimeRoot 'sidecar_server.py'
$SharedDir = Join-Path $RuntimeRoot '_shared'
if (-not (Test-Path -LiteralPath (Join-Path $SharedDir 'bgx_sidecar_common.py'))) {
    $SharedDir = Join-Path (Split-Path (Split-Path $RuntimeRoot -Parent) -Parent) '_shared'
}
function Write-BootstrapProgress {
    param([string]$Phase, [string]$Message)
    $entry = @{
        ts      = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
        phase   = $Phase
        message = $Message
    } | ConvertTo-Json -Compress
    $Utf8NoBom = New-Object System.Text.UTF8Encoding $false
    if (-not (Test-Path -LiteralPath $BootstrapJsonl)) {
        [System.IO.File]::WriteAllText($BootstrapJsonl, "$entry`n", $Utf8NoBom)
    } else {
        [System.IO.File]::AppendAllText($BootstrapJsonl, "$entry`n", $Utf8NoBom)
    }
}

# RI033 passive start (src-tauri/src/runtime_intake/passive_start.rs):
#   BGX_EXTERNAL_PROVISIONED=1 -> never touch the network (no PyPI/CDN/HF); missing wheels are
#                                reported as WHEELHOUSE_INCOMPLETE missing=<name==ver,...>.
#   BGX_PROBE_ONLY=1           -> only probe: exit 3 (PROBE_ONLY_NEEDS_INSTALL) before any install,
#                                exit 0 (PROBE_ONLY_READY) instead of launching the server.
$BgxExternalProvisioned = ($env:BGX_EXTERNAL_PROVISIONED -eq '1')
$BgxProbeOnly = ($env:BGX_PROBE_ONLY -eq '1')
if ($BgxExternalProvisioned -or $BgxProbeOnly) {
    $env:HF_HUB_OFFLINE = '1'
    $env:TRANSFORMERS_OFFLINE = '1'
    $env:PIP_NO_INDEX = '1'
}
function Write-WheelhouseIncomplete {
    param($PipOutput)
    $text = ($PipOutput | Out-String)
    $pattern = '(?:No matching distribution found for|Could not find a version that satisfies the requirement)\s+([^\s(]+)'
    $missing = @([regex]::Matches($text, $pattern) | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique)
    if ($missing.Count -gt 0) {
        Write-BootstrapProgress -Phase 'wheelhouse_incomplete' -Message ('WHEELHOUSE_INCOMPLETE missing=' + ($missing -join ','))
    }
}
function Exit-IfProbeOnly {
    param([string]$Reason)
    if ($BgxProbeOnly) {
        Write-BootstrapProgress -Phase 'probe_needs_install' -Message ('PROBE_ONLY_NEEDS_INSTALL ' + $Reason)
        exit 3
    }
}
function Exit-IfProbeReady {
    if ($BgxProbeOnly) {
        Write-BootstrapProgress -Phase 'probe_ready' -Message 'PROBE_ONLY_READY'
        exit 0
    }
}

function Write-SidecarStderr {
    param([string]$Line)
    [Console]::Error.WriteLine($Line)
}

function Resolve-Python312 {
    # INV-TTS-PY-SPAWN-01: managed only; no PATH fallback.
    $managed = $env:BGXIONG_MANAGED_PYTHON
    if ([string]::IsNullOrWhiteSpace($managed) -or -not (Test-Path -LiteralPath $managed)) {
        return $null
    }
    $verText = & $managed --version 2>&1 | Out-String
    if ($verText -match 'Python 3\.12\.') {
        return $managed
    }
    return $null
}

for ($i = 0; $i -lt $args.Count; $i++) {
    if ($args[$i] -eq '--port' -and ($i + 1) -lt $args.Count) {
        $Port = [int]$args[$i + 1]
        break
    }
}
if ($Port -le 0) {
    Write-Error 'Missing --port argument'
    exit 1
}

$env:BGX_TTS_SIDECAR_PORT = [string]$Port

if (Test-Path -LiteralPath $BootstrapJsonl) {
    Remove-Item -LiteralPath $BootstrapJsonl -Force
}
Write-BootstrapProgress -Phase 'init' -Message "Starting VoxCPM sidecar bootstrap (port=$Port)"

$pythonCmd = Resolve-Python312
if (-not $pythonCmd) {
    Write-BootstrapProgress -Phase 'error' -Message 'BGXIONG_MANAGED_PYTHON required (Python 3.12). Download managed Python runtime in Settings.'
    Write-Error 'BGXIONG_MANAGED_PYTHON required (cp312 wheelhouse).'
    exit 1
}
Write-BootstrapProgress -Phase 'python_check' -Message "Using Python: $pythonCmd"

if (-not (Test-Path -LiteralPath $VenvDir)) {
    Write-BootstrapProgress -Phase 'venv_create' -Message 'Creating virtual environment .venv...'
    & $pythonCmd -m venv $VenvDir
    if ($LASTEXITCODE -ne 0) {
        Write-BootstrapProgress -Phase 'error' -Message 'Failed to create virtual environment'
        exit $LASTEXITCODE
    }
}

$venvPython = Join-Path $VenvDir 'Scripts\python.exe'
if (-not (Test-Path -LiteralPath $venvPython)) {
    Write-BootstrapProgress -Phase 'error' -Message "Virtual environment Python not found: $venvPython"
    exit 1
}

# SharedDir = sidecar helpers; RuntimeRoot = server modules (after venv ready).
$env:PYTHONPATH = "$SharedDir;$RuntimeRoot"

function Get-RequirementsContentSha256 {
    param([string]$Path)
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant())
    } finally {
        $sha.Dispose()
    }
}

$pipMarker = Join-Path $VenvDir '.pip_installed_voxcpm_store'
$storeRoot = $env:BGX_TTS_STORE_ROOT
if ([string]::IsNullOrWhiteSpace($storeRoot)) {
    Write-BootstrapProgress -Phase 'error' -Message 'BGX_TTS_STORE_ROOT not set - run Rust predownload_tts_artifacts first'
    Write-Error 'BGX_TTS_STORE_ROOT not set'
    exit 1
}
$wheelhouse = Join-Path $storeRoot 'wheelhouse'

$requirementsHash = Get-RequirementsContentSha256 -Path $Requirements
$needsPipInstall = $true
if (Test-Path -LiteralPath $pipMarker) {
    $markerHash = (Get-Content -LiteralPath $pipMarker -Raw -Encoding ASCII).Trim()
    if ($markerHash -eq $requirementsHash) {
        $needsPipInstall = $false
    }
}
if ($needsPipInstall) {
    if (-not (Test-Path -LiteralPath $wheelhouse)) {
        Write-BootstrapProgress -Phase 'manual_required' -Message "wheelhouse missing at $wheelhouse - predownload via client first"
        exit 1
    }
    Write-BootstrapProgress -Phase 'pip_install' -Message "pip install --no-index --find-links $wheelhouse (requirements hash changed or first install)"
    Exit-IfProbeOnly 'pip install from wheelhouse (requirements hash changed or first install)'
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $pipOut = & $venvPython -m pip install --no-index --find-links $wheelhouse -r $Requirements 2>&1
    $pipExit = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    $pipOut | ForEach-Object { [Console]::Error.WriteLine("$_") }
    if ($pipExit -ne 0) { Write-WheelhouseIncomplete $pipOut }
    if ($pipExit -ne 0) {
        Write-BootstrapProgress -Phase 'error' -Message "pip install failed exit=$pipExit. See $StderrLog"
        exit $pipExit
    }
    Set-Content -LiteralPath $pipMarker -Value $requirementsHash -Encoding ASCII
    Write-BootstrapProgress -Phase 'pip_done' -Message 'pip dependencies installed from local wheelhouse'
} else {
    Write-BootstrapProgress -Phase 'pip_skip' -Message 'pip already installed (requirements hash match)'
}

Set-Content -LiteralPath $PidFile -Value $PID -Encoding ASCII
Write-BootstrapProgress -Phase 'launch' -Message 'Starting sidecar_server.py...'
Write-SidecarStderr "sidecar_server starting python=$venvPython port=$Port"

Exit-IfProbeReady
& $venvPython $ServerPy
$exitCode = $LASTEXITCODE
if ($exitCode -ne 0) {
    Write-BootstrapProgress -Phase 'error' -Message "sidecar_server exited with code=$exitCode. See $StderrLog"
}
exit $exitCode
