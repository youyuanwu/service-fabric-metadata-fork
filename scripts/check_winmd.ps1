#!/usr/bin/env pwsh
param(
    [string]$WinmdPath = ".windows/winmd/Microsoft.ServiceFabric.winmd",
    [switch]$MigrationBaseline
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
if (-not (Test-Path (Join-Path $repoRoot '.git'))) {
    Write-Error "Repository root not found at $repoRoot"
    exit 1
}
Set-Location $repoRoot

if (-not (Test-Path $WinmdPath)) {
    Write-Error "Winmd file not found: $WinmdPath"
    exit 1
}

$tempDir = Join-Path $env:TEMP "winmd_check_$(Get-Random)"
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

try {
    $gitPath = $WinmdPath -replace '\\', '/'
    $oldWinmdPath = Join-Path $tempDir "old.winmd"
    git show "HEAD:$gitPath" > $oldWinmdPath
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $oldWinmdPath)) {
        Write-Error "Failed to extract winmd from git"
        exit 1
    }
    $arguments = @(
        'run',
        '--manifest-path', 'rust-metadata/Cargo.toml',
        '--release',
        '--locked',
        '--bin', 'validate_winmd',
        '--'
    )
    if ($MigrationBaseline) {
        $arguments += '--migration-baseline'
    }
    $arguments += @($oldWinmdPath, $WinmdPath)
    cargo @arguments
    exit $LASTEXITCODE
} finally {
    if (Test-Path $tempDir) {
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
