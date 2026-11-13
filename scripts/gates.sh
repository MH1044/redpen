#!/bin/sh
# Quality gates (docs/TESTING.md §4): sh scripts/gates.sh <gate> [options].
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

set -eu
cd "$(dirname "$0")/.."

usage() {
  echo "Usage: sh scripts/gates.sh <gate> [options]"
  echo "Gates: check-speed, eval-rewrite"
}

gate="${1:-}"
[ $# -gt 0 ] && shift

case "$gate" in
  check-speed)
    exec cargo run --release -q -p redpen-cli -- check-speed "$@" tests/cases/01-harper-check/corpus.jsonl
    ;;
  eval-rewrite)
    # A debug build: the calls take the time, and the question comes without a release build.
    exec cargo run -q -p redpen-cli -- eval-rewrite "$@" tests/cases/04-rewrite-engine/fix-grammar.jsonl tests/cases/04-rewrite-engine/languages.jsonl
    ;;
  -h | --help)
    usage
    ;;
  "")
    usage >&2
    exit 2
    ;;
  *)
    echo "error: unknown gate $gate." >&2
    usage >&2
    exit 2
    ;;
esac
