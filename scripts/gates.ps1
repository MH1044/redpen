# Quality gates (docs/TESTING.md §4) for Windows PowerShell 5.1: the same gates as
# scripts/gates.sh. powershell -File scripts/gates.ps1 <gate> [options]
#
#   check-speed   free. Builds the release `redpen` and times Harper on a 1,000-word text made
#                 from the module 01 corpus: the median of 10 checks, each on a fresh checker so
#                 no run reuses the last one's cache. Exits 1 when the median is above 50 ms.
#                 Options are passed on, e.g. `--runs 20` or `--dialect british`.
#
# eval-rewrite (module 04) and eval-plagiarism (module 09) arrive with their modules. They spend
# money: they print the number of calls and the tier and ask before sending anything.
#
# Native stderr is left alone: piping it through 2>&1 would make a success look like a failure.
# The arguments are read from $args rather than a param block, so options such as --runs and
# --help reach the gate as they were typed instead of being taken for PowerShell parameters.

$ErrorActionPreference = 'Continue'
Set-Location (Join-Path $PSScriptRoot '..')

$Gate = ''
$Options = @()
if ($args.Count -gt 0) { $Gate = [string]$args[0] }
if ($args.Count -gt 1) { $Options = @($args[1..($args.Count - 1)] | ForEach-Object { [string]$_ }) }

function Show-Usage {
    Write-Host 'Usage: powershell -File scripts/gates.ps1 <gate> [options]'
    Write-Host 'Gates: check-speed'
}

switch -Exact ($Gate) {
    'check-speed' {
        $arguments = @('run', '--release', '-q', '-p', 'redpen-cli', '--', 'check-speed') + $Options + @('tests/cases/01-harper-check/corpus.jsonl')
        & cargo @arguments
        exit $LASTEXITCODE
    }
    { $_ -in @('-h', '--help') } {
        Show-Usage
        exit 0
    }
    '' {
        Show-Usage
        exit 2
    }
    default {
        Write-Host "error: unknown gate $Gate."
        Show-Usage
        exit 2
    }
}
