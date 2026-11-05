param()
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$versionLine = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') | Where-Object { $_ -match '^version = "([^"]+)"$' } | Select-Object -First 1
if ($versionLine -notmatch '^version = "([^"]+)"$') { throw 'Missing package version' }
$version = $Matches[1]
$out = Join-Path $root ('install-' + $version)
if (Test-Path -LiteralPath $out) { throw "Refusing to overwrite distribution: $out" }
$wasm = Join-Path $root 'target/wasm32-wasip2/release/h3_face_refine_adapter.wasm'
if (-not (Test-Path -LiteralPath $wasm)) { throw 'Build wasm32-wasip2 release first' }
$manifestPath = Join-Path $root 'manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$manifest.version = $version
$manifest.artifactSha256 = (Get-FileHash -LiteralPath $wasm -Algorithm SHA256).Hash.ToLowerInvariant()
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 40) + "`n", $utf8)
Copy-Item -LiteralPath $wasm -Destination (Join-Path $root 'adapter.wasm')
$manifestHash = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
$inventory = "$manifestHash manifest.json`n$($manifest.artifactSha256) adapter.wasm`n"
[IO.File]::WriteAllText((Join-Path $root 'manifest.sha256'), $inventory, $utf8)
$sourcePath = Join-Path $root 'SOURCE.json'
$sourceRecord = Get-Content -LiteralPath $sourcePath -Raw | ConvertFrom-Json
$sourceRecord.version = $version
[IO.File]::WriteAllText($sourcePath, ($sourceRecord | ConvertTo-Json -Depth 40) + "`n", $utf8)
New-Item -ItemType Directory -Path (Join-Path $out 'package') -Force | Out-Null
foreach ($name in @('manifest.json','manifest.sha256','adapter.wasm')) { Copy-Item -LiteralPath (Join-Path $root $name) -Destination (Join-Path $out 'package') }
$code = [Collections.Generic.List[object]]::new()
foreach ($mount in @(@{source='companion/upstream/ComfyUI-H3-FaceRefine'; name='ComfyUI-H3-FaceRefine'}, @{source='companion/custom_nodes/BGXFaceRefineContract'; name='BGXFaceRefineContract'})) {
    $source = (Resolve-Path -LiteralPath (Join-Path $root $mount.source)).Path
    foreach ($file in Get-ChildItem -LiteralPath $source -File -Recurse) {
        $relative = $file.FullName.Substring($source.Length + 1).Replace('\','/')
        if ($relative -match '(^|/)(__pycache__|\.git)(/|$)' -or $relative.EndsWith('.pyc')) { continue }
        $path = 'comfy-nodes/' + $mount.name + '/' + $relative
        $dest = Join-Path $out $path
        New-Item -ItemType Directory -Path (Split-Path $dest -Parent) -Force | Out-Null
        Copy-Item -LiteralPath $file.FullName -Destination $dest
        $code.Add(@{path=$path; sha256=(Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash.ToLowerInvariant()})
    }
}
$descriptor = @{
    schemaVersion=1; packageDirectory='package'; manifestSha256=$manifestHash
    authentication=@{source='none'}; managedMediaProbe=$true
    comfyui=@{customNodes='comfy-nodes'; files=@($code.ToArray() | Sort-Object path)}
    hostConfig=@{
        bindingId='h3-face-refine-saved-comfy'; bindingRevision=1; configurationRef='h3-face-refine-saved-comfy-v1'
        configuration=@{timeoutMs=600000}
        network=@{origins=@(); allowPrivate=$true; allowEmbeddedAuthorization=$false; deniedHosts=@(); deniedNetworks=@()}
        credentials=@(); trust=@{publishers=@(); revokedPlugins=@(); revokedVersions=@(); revokedKeys=@(); denylistedHosts=@(); isFresh=$false; supportedFeatures=@('media-target-selection-v1','submission-rejection-v1')}
        mediaTtlMs=3600000; transport=@{handshakeTimeoutMs=10000; writeTimeoutMs=60000; shutdownTimeoutMs=10000; cancelGraceMs=100}
        budgetMs=@{capabilities=60000; probe=60000; submit=600000; query=600000; cancel=60000}; probe=$null
    }
}
[IO.File]::WriteAllText((Join-Path $out 'adapter-install.json'), ($descriptor | ConvertTo-Json -Depth 40) + "`n", $utf8)
Write-Output "INSTALL_PACKAGE=$out NODE_FILES=$($code.Count) VERSION=$version"
