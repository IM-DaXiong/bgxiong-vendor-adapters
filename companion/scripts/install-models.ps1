param([Parameter(Mandatory = $true)][string]$ModelsRoot)
$ErrorActionPreference = "Stop"
$Companion = Split-Path $PSScriptRoot -Parent
$ModelManifest = Get-Content -LiteralPath (Join-Path $Companion "models.json") -Raw | ConvertFrom-Json
$ModelsAbsolute = [IO.Path]::GetFullPath($ModelsRoot)
foreach ($model in $ModelManifest.models) {
  $destination = [IO.Path]::GetFullPath((Join-Path $ModelsAbsolute $model.path))
  if (-not $destination.StartsWith($ModelsAbsolute.TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw "Model path escapes model directory" }
  if (Test-Path -LiteralPath $destination) {
    if ((Get-Item -LiteralPath $destination).Length -ne $model.bytes -or (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant() -ne $model.sha256) { throw "Existing model differs from pinned package asset: $destination" }
    Write-Host "MODEL_VERIFIED $destination"
    continue
  }
  New-Item -ItemType Directory -Force -Path (Split-Path $destination -Parent) | Out-Null
  $temporary = $destination + ".download-" + [Guid]::NewGuid().ToString('N')
  try {
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -UseBasicParsing -Uri $model.url -OutFile $temporary
    if ((Get-Item -LiteralPath $temporary).Length -ne $model.bytes -or (Get-FileHash -LiteralPath $temporary -Algorithm SHA256).Hash.ToLowerInvariant() -ne $model.sha256) { throw "Downloaded model failed pinned size/SHA256 verification" }
    Move-Item -LiteralPath $temporary -Destination $destination
    Write-Host "MODEL_INSTALLED $destination sha256=$($model.sha256)"
  } finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
  }
}
