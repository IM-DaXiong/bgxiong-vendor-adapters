param(
  [Parameter(Mandatory = $true)][string]$BaseUrl
)
$ErrorActionPreference = "Stop"
$BaseUrl = $BaseUrl.TrimEnd('/')
$required = @("H3FaceSelect","H3FaceTrackCrop","H3FaceStitch","H3InjectVideoLatent","H3PerFrameDenoise")
$info = Invoke-RestMethod -Uri "$BaseUrl/object_info" -Method GET
$missing = @()
foreach ($c in $required) {
  if (-not $info.PSObject.Properties.Name.Contains($c)) { $missing += $c }
}
try {
  $null = Invoke-RestMethod -Uri "$BaseUrl/h3_facerefine/videos" -Method GET
  $picker = $true
} catch { $picker = $false }
$result = [ordered]@{
  ok = ($missing.Count -eq 0 -and $picker)
  missingClasses = $missing
  pickerRoute = $picker
  baseUrl = $BaseUrl
}
$result | ConvertTo-Json -Compress
if (-not $result.ok) { exit 2 }
