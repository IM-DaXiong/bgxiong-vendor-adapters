param([Parameter(Mandatory=$true)][string]$Task, [string]$PythonExecutable = 'python',
      [string]$BaselinePath, [string]$AuthorEvidenceRoot, [string]$AuthorProof,
      [string[]]$ArtifactRoot = @())
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$sdkRoot = Split-Path $PSScriptRoot -Parent
$repoRoot = [IO.Path]::GetFullPath((Join-Path $sdkRoot '../../..'))
$checks = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'task-checks.json') | ConvertFrom-Json -AsHashtable
if (-not $checks.tasks.ContainsKey($Task)) { Write-Error "UNKNOWN_TASK: $Task"; exit 2 }
foreach ($relative in $checks.tasks[$Task].requiredPaths) {
    if (-not (Test-Path -LiteralPath (Join-Path $sdkRoot $relative) -PathType Leaf)) { Write-Error "NOT_IMPLEMENTED: $Task missing $relative"; exit 3 }
}
function Run-Checked([string]$Executable, [string[]]$Arguments) {
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) { throw "CHECK_FAILED: $Executable (exit $LASTEXITCODE)" }
}
switch ($Task) {
    'SD00' {
        if (-not $BaselinePath) { $BaselinePath = Join-Path $repoRoot 'docs/dev/reports/sdk-v2-development/baseline.json' }
        Run-Checked $PythonExecutable @((Join-Path $PSScriptRoot 'check_isolation.py'), '--baseline', $BaselinePath)
    }
    'SD01' {
        $testRoot = Join-Path ([IO.Path]::GetTempPath()) ('bgx-sdk-workspace-' + [Guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $testRoot | Out-Null
        try {
            # Later tasks add nested author/compiler caches. Copy only ordinary
            # public files and reject links before descending, at every depth.
            $copyRoot = Join-Path $testRoot 'sdk'
            New-Item -ItemType Directory -Path $copyRoot | Out-Null
            function Copy-PublicSource([string]$Source, [string]$Destination) {
                foreach ($item in Get-ChildItem -LiteralPath $Source -Force) {
                    if ($item.Name -in @('target', '.sdk-test-workspaces', '.git', '.bgx-sdk', 'node_modules', '__pycache__')) { continue }
                    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'public source copy refuses reparse points' }
                    $target = Join-Path $Destination $item.Name
                    if ($item.PSIsContainer) {
                        New-Item -ItemType Directory -Path $target | Out-Null
                        Copy-PublicSource $item.FullName $target
                    } else { Copy-Item -LiteralPath $item.FullName -Destination $target }
                }
            }
            Copy-PublicSource $sdkRoot $copyRoot
            $manifest = Join-Path $copyRoot 'Cargo.toml'
            $json = & cargo metadata --locked --offline --format-version 1 --no-deps --manifest-path $manifest
            if ($LASTEXITCODE -ne 0) { throw 'independent cargo metadata failed' }
            $metadata = $json | ConvertFrom-Json
            if ($metadata.workspace_members.Count -ne 7) { throw 'workspace member count mismatch' }
            foreach ($package in $metadata.packages) {
                foreach ($dependency in $package.dependencies) {
                    if ($dependency.path -and -not ([IO.Path]::GetFullPath($dependency.path)).StartsWith([IO.Path]::GetFullPath($copyRoot) + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
                        throw 'path dependency outside public SDK'
                    }
                }
            }
            Run-Checked 'cargo' @('check', '--workspace', '--locked', '--offline', '--manifest-path', $manifest)
            & pwsh -NoProfile -File $PSCommandPath -Task 'SD99' *> (Join-Path $testRoot 'unknown-task.log')
            if ($LASTEXITCODE -eq 0) { throw 'unknown task was accepted' }
            # Deliberately break this owned copy, even when SD32 is implemented.
            $missingFixture = Join-Path $copyRoot 'schemas/adoption-evidence.schema.json'
            if (Test-Path -LiteralPath $missingFixture -PathType Leaf) { Remove-Item -LiteralPath $missingFixture }
            & pwsh -NoProfile -File (Join-Path $copyRoot 'tools/check_sdk.ps1') -Task 'SD32' *> (Join-Path $testRoot 'not-implemented.log')
            if ($LASTEXITCODE -eq 0) { throw 'unimplemented task was accepted' }
            Write-Output 'SD01_LOCAL=PASS independent_copy_build=PASS unknown_task_rejected=PASS unimplemented_task_rejected=PASS'
        } finally {
            # Only remove the explicitly verified, newly created temp workspace.
            $resolved = [IO.Path]::GetFullPath($testRoot)
            $tempPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
            if (-not $resolved.StartsWith($tempPrefix, [StringComparison]::OrdinalIgnoreCase) -or -not (Split-Path $resolved -Leaf).StartsWith('bgx-sdk-workspace-')) { throw 'unsafe test cleanup path' }
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
    'SD30' {
        if (-not $AuthorEvidenceRoot -or -not $AuthorProof) { throw 'SD30 requires explicit AuthorEvidenceRoot and AuthorProof from the independent author run' }
        $arguments = @((Join-Path $PSScriptRoot 'check_author_evidence.py'), '--evidence-root', $AuthorEvidenceRoot, '--proof', $AuthorProof)
        foreach ($root in $ArtifactRoot) { $arguments += @('--artifact-root', $root) }
        Run-Checked $PythonExecutable $arguments
    }
    default {
        if ($checks.tasks[$Task].commands.Count -eq 0) { throw "NOT_IMPLEMENTED: $Task has no checks" }
        foreach ($command in $checks.tasks[$Task].commands) {
            $expanded = @($command | ForEach-Object { $_.Replace('{sdk}', $sdkRoot).Replace('{repo}', $repoRoot).Replace('{python}', $PythonExecutable) })
            Run-Checked $expanded[0] @($expanded | Select-Object -Skip 1)
        }
    }
}
