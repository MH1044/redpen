# The one full check (docs/TESTING.md §1) for Windows PowerShell 5.1: the same steps, in the same
# order, as scripts/verify.sh. Stops at the first failure.
#
# PowerShell 5.1 has no && operator, so every step reads $LASTEXITCODE; native stderr is left
# alone (piping it through 2>&1 would make a success look like a failure).

$ErrorActionPreference = 'Continue'
Set-Location (Join-Path $PSScriptRoot '..')

function Step($name, $command, $arguments) {
    Write-Host ""
    Write-Host "== $name"
    & $command @arguments
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Host "verify: '$name' failed with exit code $LASTEXITCODE"
        exit $LASTEXITCODE
    }
}

# npm ci when the dependencies are missing or the lockfile changed since they were installed.
$installed = 'app/node_modules/.package-lock.json'
$lock = 'app/package-lock.json'
if (-not (Test-Path $installed) -or ((Get-Item $lock).LastWriteTime -gt (Get-Item $installed).LastWriteTime)) {
    Step 'npm ci' 'npm' @('ci', '--prefix', 'app')
}
Step 'front-end build' 'npm' @('--prefix', 'app', 'run', 'build')
Step 'cargo fmt --check' 'cargo' @('fmt', '--all', '--', '--check')
Step 'clippy' 'cargo' @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')
Step 'typecheck' 'npm' @('--prefix', 'app', 'run', 'typecheck')
Step 'lint' 'npm' @('--prefix', 'app', 'run', 'lint')
Step 'format check' 'npm' @('--prefix', 'app', 'run', 'format:check')
Step 'licence gate: cargo deny' 'cargo' @('deny', 'check', 'licenses')
Step 'cargo test' 'cargo' @('test', '--workspace')
Step 'vitest' 'npm' @('--prefix', 'app', 'test')

Write-Host ""
Write-Host "verify: all green"
