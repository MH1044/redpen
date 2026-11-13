# Quality gates (docs/TESTING.md §4) for Windows PowerShell 5.1: the same gates as
# scripts/gates.sh. powershell -File scripts/gates.ps1 <gate> [options]
#
#   check-speed   free. Builds the release `redpen` and times Harper on a 1,000-word text made
#                 from the module 01 corpus: the median of 10 checks, each on a fresh checker so
#                 no run reuses the last one's cache. Exits 1 when the median is above 50 ms.
#                 Options are passed on, e.g. `--runs 20` or `--dialect british`.
#
#   eval-rewrite  spends money (about $0.44 on publik API; the maintainer's to run). Fix grammar
#                 on the 100 sentences of the module 04 fix-grammar.jsonl (median edit ratio at
#                 most 0.15, guard warnings on at most 5% of answers) and five rewrites of each
#                 paragraph of the module 04 languages.jsonl (language kept in at least 98%), 300
#                 calls in all. It prints the number of calls and the tiers and asks y/N before
#                 sending anything; after the first call it prints what that call cost. publik
#                 API by default (PUBLIK_API_KEY); pass `--provider own-key` for the own key
#                 (REDPEN_OWN_KEY, REDPEN_OWN_BASE_URL, REDPEN_OWN_MODEL). Exits 1 when a
#                 threshold does not hold.
#
# eval-plagiarism (module 09) arrives with its module. It spends money too, and asks the same way.
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
    Write-Host 'Gates: check-speed, eval-rewrite'
}

switch -Exact ($Gate) {
    'check-speed' {
        $arguments = @('run', '--release', '-q', '-p', 'redpen-cli', '--', 'check-speed') + $Options + @('tests/cases/01-harper-check/corpus.jsonl')
        & cargo @arguments
        exit $LASTEXITCODE
    }
    'eval-rewrite' {
        # A debug build: the calls take the time, and the question comes without a release build.
        $arguments = @('run', '-q', '-p', 'redpen-cli', '--', 'eval-rewrite') + $Options + @('tests/cases/04-rewrite-engine/fix-grammar.jsonl', 'tests/cases/04-rewrite-engine/languages.jsonl')
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
