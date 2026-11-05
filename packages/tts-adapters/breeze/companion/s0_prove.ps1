# Breeze TTS 2 S0 prove - Windows x64 managed CPython3.12 CUDA eager
# Stages under E:\BreezeTTS2-assets\s0-staging; never overwrites app runtimes store.
param(
  [string]$AssetsRoot = 'E:\BreezeTTS2-assets',
  [string]$ManagedPython = $env:BGXIONG_MANAGED_PYTHON,
  [string]$RepoBreezeRoot = ''
)
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepoBreezeRoot)) { $RepoBreezeRoot = Join-Path $PSScriptRoot '.' }
$RepoBreezeRoot = (Resolve-Path -LiteralPath $RepoBreezeRoot).Path
$Staging = Join-Path $AssetsRoot 's0-staging'
$VenvDir = Join-Path $Staging 'venv'
$Wheelhouse = Join-Path $AssetsRoot 'wheelhouse\cp312-win_amd64'
$UpstreamSrc = Join-Path $AssetsRoot 'upstream\breeze-tts-6eba901d'
$OutDir = Join-Path $Staging 'out'
$LogDir = Join-Path $Staging 'logs'
New-Item -ItemType Directory -Force -Path $Staging, $OutDir, $LogDir | Out-Null
function Fail([string]$Msg) { Write-Host ('S0_BLOCKED: ' + $Msg); exit 2 }
if (-not (Test-Path -LiteralPath $AssetsRoot)) { Fail ('assets root missing: ' + $AssetsRoot) }
if ([string]::IsNullOrWhiteSpace($ManagedPython) -or -not (Test-Path -LiteralPath $ManagedPython)) {
  $bundled = Join-Path $AssetsRoot 'managed-python\python.exe'
  if (Test-Path -LiteralPath $bundled) { $ManagedPython = $bundled }
}
if ([string]::IsNullOrWhiteSpace($ManagedPython) -or -not (Test-Path -LiteralPath $ManagedPython)) { Fail 'BGXIONG_MANAGED_PYTHON (cp312) not set/found' }
$ver = & $ManagedPython --version 2>&1 | Out-String
if ($ver -notmatch 'Python 3\.12\.') { Fail ('managed python is not 3.12: ' + $ver) }
$CkptDir = $null
foreach ($c in @((Join-Path $AssetsRoot 'checkpoints\Breeze-TTS-2-3e28c515'), (Join-Path $AssetsRoot 'checkpoints'))) {
  if ((Test-Path -LiteralPath (Join-Path $c 'config.json')) -and (Test-Path -LiteralPath (Join-Path $c 'model.safetensors.index.json'))) { $CkptDir = $c; break }
}
if (-not $CkptDir) { Fail 'checkpoints incomplete' }
if (-not (Test-Path -LiteralPath (Join-Path $CkptDir 'model-00001-of-00002.safetensors'))) { Fail 'shard1 missing/partial' }
if (-not (Test-Path -LiteralPath (Join-Path $CkptDir 'model-00002-of-00002.safetensors'))) { Fail 'shard2 missing/partial' }
if (-not (Test-Path -LiteralPath $Wheelhouse)) { Fail ('wheelhouse missing: ' + $Wheelhouse) }
$wheelCount = @(Get-ChildItem -LiteralPath $Wheelhouse -File -Filter *.whl -ErrorAction SilentlyContinue).Count
if ($wheelCount -lt 5) { Fail ('wheelhouse too few wheels: ' + $wheelCount) }
if (-not (Test-Path -LiteralPath (Join-Path $UpstreamSrc 'infer.py'))) { Fail 'upstream infer.py missing' }
if (-not (Test-Path -LiteralPath (Join-Path $VenvDir 'Scripts\python.exe'))) { & $ManagedPython -m venv $VenvDir }
$py = Join-Path $VenvDir 'Scripts\python.exe'
$req = Join-Path $RepoBreezeRoot 'requirements.txt'
Write-Host ('S0 pip install --no-index from ' + $Wheelhouse)
& $py -m pip install --upgrade pip
& $py -m pip install --no-index --find-links $Wheelhouse -r $req
if ($LASTEXITCODE -ne 0) { Fail ('pip install failed exit=' + $LASTEXITCODE) }
& $py -m pip check
if ($LASTEXITCODE -ne 0) { Fail 'pip check failed' }
$env:PYTHONPATH = $UpstreamSrc + ';' + $RepoBreezeRoot
$probePath = Join-Path $LogDir 'cuda_import_probe.py'
$probe = "import torch
print(torch.__version__, torch.cuda.is_available(), torch.cuda.device_count())
assert torch.cuda.is_available()
print(torch.cuda.get_device_name(0))
from breeze_infer.runtime import resolve_device
print(resolve_device())
"
[System.IO.File]::WriteAllText($probePath, $probe, (New-Object System.Text.UTF8Encoding $false))
& $py $probePath
if ($LASTEXITCODE -ne 0) { Fail 'CUDA/import probe failed' }
$RefWav = Join-Path $AssetsRoot 'ref\s0_ref_zh.wav'
$RefText = 'This is the reference transcript for clone.'
if (-not (Test-Path -LiteralPath $RefWav)) {
  New-Item -ItemType Directory -Force -Path (Split-Path $RefWav) | Out-Null
  $genPath = Join-Path $LogDir 'gen_ref.py'
  $gen = "import numpy as np, soundfile as sf, sys
sr=24000
t=np.linspace(0,1.2,int(sr*1.2),endpoint=False)
sf.write(sys.argv[1], (0.2*np.sin(2*np.pi*220*t)).astype(np.float32), sr, subtype='PCM_16')
"
  [System.IO.File]::WriteAllText($genPath, $gen, (New-Object System.Text.UTF8Encoding $false))
  & $py $genPath $RefWav
  Write-Host 'WARN: synthetic ref WAV; replace with real speech+accurate transcript for listen QA'
}
$infer = Join-Path $UpstreamSrc 'infer.py'
function Run-Mode([string]$Name, [string[]]$ExtraArgs) {
  $out = Join-Path $OutDir ('s0_' + $Name + '.wav')
  Write-Host ('S0 RUN ' + $Name + ' -> ' + $out)
  & $py $infer $CkptDir @ExtraArgs --output $out --seed 42
  if ($LASTEXITCODE -ne 0) { Fail ('upstream CLI mode ' + $Name + ' failed exit=' + $LASTEXITCODE) }
  if (-not (Test-Path -LiteralPath $out) -or ((Get-Item $out).Length -lt 1000)) { Fail ('mode ' + $Name + ' produced missing/too-small WAV') }
  Write-Host ('S0 OK ' + $Name + ' bytes=' + (Get-Item $out).Length)
}
Run-Mode 'plain' @('--text', 'Hello, this is Breeze plain reading.')
Run-Mode 'design' @('--text', 'Welcome back to the story.', '--instruction', 'A calm female narrator, medium pace')
Run-Mode 'clone' @('--text', 'We continue the story.', '--ref-audio', $RefWav, '--ref-text', $RefText)
Run-Mode 'direction' @('--text', 'Please speak more gently.', '--ref-audio', $RefWav, '--ref-text', $RefText, '--instruction', 'gentle and calm', '--cfg-scale', '4')
Write-Host ('S0_PASS: four WAV files under ' + $OutDir)
Get-ChildItem -LiteralPath $OutDir -Filter 's0_*.wav' | ForEach-Object { Write-Host $_.FullName $_.Length }
exit 0