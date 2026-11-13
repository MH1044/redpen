# Maintainer only: one real publik API call (docs/modules/03-publik.md, contract §11), for
# Windows PowerShell 5.1: the same as scripts/publik-proof.sh. It spends money and is never run
# by the tests or CI. powershell -File scripts/publik-proof.ps1 [--models] [--probe-json] [--search]
#
#   (no option)    one publik-fast call ("Reply with the word ok."): prints the reply, the
#                  x-publik-charge-micros line and the names of the x-publik-* headers received
#   --models       the model ids of GET /models
#   --probe-json   whether response_format {"type": "json_object"} is honoured
#   --search       one web search (one cent, after its own y/N) for "largest coral reef system":
#                  the first result's title and site, and the charge
#
# Without an app token it prints "This build has no publik API app token. Use your own key for
# now." and exits 2, sending nothing. With no publik key on this computer it shows the
# disclosure and asks y/N before setting it up. It never prints the key or the balance.
#
# Native stderr is left alone: piping it through 2>&1 would make a success look like a failure.
# The arguments are read from $args, so --models and the others reach the proof as typed.

$ErrorActionPreference = 'Continue'
Set-Location (Join-Path $PSScriptRoot '..')

$Options = @($args | ForEach-Object { [string]$_ })

# The app crate embeds the front end at compile time, so it must exist before cargo builds.
if (-not (Test-Path 'app/dist/index.html')) {
    if (-not (Test-Path 'app/node_modules')) {
        & npm ci --prefix app
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    & npm --prefix app run build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

$arguments = @('run', '-q', '-p', 'redpen-app', '--bin', 'publik-proof', '--') + $Options
& cargo @arguments
exit $LASTEXITCODE
