# Development roots are discovered from the unified checkout, never a drive letter.
function Get-AdapterDevelopmentRoots([string]$FromDirectory) {
    $cursor = Get-Item -LiteralPath ([IO.Path]::GetFullPath($FromDirectory))
    while ($cursor) {
        if ((Test-Path -LiteralPath (Join-Path $cursor.FullName '.git')) -and $cursor.Name -eq 'bgxiong-third-party-adapters') {
            $hostRoot = Join-Path $cursor.Parent.FullName 'bgxiong-ai-story'
            if (-not (Test-Path -LiteralPath (Join-Path $hostRoot 'packages/vendor-adapter-contract/v2/Cargo.toml'))) { throw "Missing sibling host SDK: $hostRoot" }
            return [pscustomobject]@{ AdapterRoot=$cursor.FullName; HostRoot=[IO.Path]::GetFullPath($hostRoot) }
        }
        $cursor = $cursor.Parent
    }
    throw 'Run this development exporter from the unified bgxiong-third-party-adapters checkout.'
}
