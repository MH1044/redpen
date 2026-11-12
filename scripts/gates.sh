#!/bin/sh
# Quality gates (docs/TESTING.md §4): sh scripts/gates.sh <gate> [options].
#
#   check-speed   free. Builds the release `redpen` and times Harper on a 1,000-word text made
#                 from the module 01 corpus: the median of 10 checks, each on a fresh checker so
#                 no run reuses the last one's cache. Exits 1 when the median is above 50 ms.
#                 Options are passed on, e.g. `--runs 20` or `--dialect british`.
#
# eval-rewrite (module 04) and eval-plagiarism (module 09) arrive with their modules. They spend
# money: they print the number of calls and the tier and ask before sending anything.

set -eu
cd "$(dirname "$0")/.."

usage() {
  echo "Usage: sh scripts/gates.sh <gate> [options]"
  echo "Gates: check-speed"
}

gate="${1:-}"
[ $# -gt 0 ] && shift

case "$gate" in
  check-speed)
    exec cargo run --release -q -p redpen-cli -- check-speed "$@" tests/cases/01-harper-check/corpus.jsonl
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
