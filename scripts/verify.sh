#!/bin/sh
# The one full check (docs/TESTING.md §1): exactly what CI runs, minus the end-to-end launch
# smoke. Green before every commit. Stops at the first failure.
#
# The front end is built first because the Tauri crate embeds app/dist at compile time, so every
# cargo step needs it to exist.

set -eu
cd "$(dirname "$0")/.."

step() {
  printf '\n== %s\n' "$*"
}

# npm ci when the dependencies are missing or the lockfile changed since they were installed.
if [ ! -f app/node_modules/.package-lock.json ] || [ app/package-lock.json -nt app/node_modules/.package-lock.json ]; then
  step "npm ci"
  npm ci --prefix app
fi

step "front-end build"
npm --prefix app run build

step "cargo fmt --check"
cargo fmt --all -- --check

step "clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "typecheck"
npm --prefix app run typecheck

step "lint"
npm --prefix app run lint

step "format check"
npm --prefix app run format:check

step "licence gate: cargo deny"
cargo deny check licenses

step "cargo test"
cargo test --workspace

step "vitest"
npm --prefix app test

printf '\nverify: all green\n'
