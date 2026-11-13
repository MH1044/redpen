#!/bin/sh
# Maintainer only: one real publik API call (docs/modules/03-publik.md, contract §11). It spends
# money and is never run by the tests or CI. sh scripts/publik-proof.sh [--models] [--probe-json] [--search]
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

set -eu
cd "$(dirname "$0")/.."

# The app crate embeds the front end at compile time, so it must exist before cargo builds.
if [ ! -f app/dist/index.html ]; then
  if [ ! -d app/node_modules ]; then
    npm ci --prefix app
  fi
  npm --prefix app run build
fi

exec cargo run -q -p redpen-app --bin publik-proof -- "$@"
