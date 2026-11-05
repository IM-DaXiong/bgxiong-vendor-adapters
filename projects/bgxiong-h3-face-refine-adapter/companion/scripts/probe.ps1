param(
  [Parameter(Mandatory = $true)][string]$BaseUrl
)
$ErrorActionPreference = "Stop"
$BaseUrl = $BaseUrl.TrimEnd('/')
$required = @("H3FaceSelect","H3FaceTrackCrop","H3FaceStitch","H3InjectVideoLatent","H3PerFrameDenoise","H3FaceTrackingReceipt")
$info = Invoke-RestMethod -Uri "$BaseUrl/object_info" -Method GET
$missing = @()
foreach ($c in $required) {
  if (-not $info.PSObject.Properties.Name.Contains($c)) { $missing += $c }
}
try {
  $null = Invoke-RestMethod -Uri "$BaseUrl/h3_facerefine/videos" -Method GET
  $picker = $true
} catch { $picker = $false }
$health = Invoke-RestMethod -Uri "$BaseUrl/bgx_face_refine/health" -Method GET
$contract = ($health.version -eq "1.0.0-a09" -and $health.analysisSessions -eq $true -and $health.trackingReceipt -eq $true)
$result = [ordered]@{
  ok = ($missing.Count -eq 0 -and $picker -and $contract)
  missingClasses = $missing
  pickerRoute = $picker
  contractRoutes = $contract
  baseUrl = $BaseUrl
}
$result | ConvertTo-Json -Compress
if (-not $result.ok) { exit 2 }
