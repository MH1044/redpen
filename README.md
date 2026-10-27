# Redpen

Fix or rewrite your writing in any app on Windows and Linux: select text and press a hotkey for
instant local grammar checks, AI rewrites on request, and a plagiarism check with citations. Free
and open source. Nothing you type leaves your computer unless you ask for a rewrite, or for a
check that needs the web.

**Status: pre-alpha.** Nothing checks or rewrites yet. The repository, the build, the tests and
continuous integration exist, and the tray app starts with a Quit menu. `docs/ROADMAP.md` lists
what comes next, in order; `docs/PRODUCT.md` describes the finished app.

## What it replaces

Redpen replaces Grammarly and QuillBot for checking and rewriting, without a subscription and
without routing everything you type through someone's servers. The always-on part is Harper, a
grammar checker that runs on this computer and needs no account, key or connection. A model is
only called on an explicit action, a rewrite button or Plagiarism check, through the publik API,
your own key, or a local model, and every AI answer is checked before you see it. A paragraph
rewrite costs about $0.002 on publik API and a plagiarism check at most $0.08
(`docs/PUBLIK-INTEGRATION.md` §8).

There is no macOS build: GSpot already covers macOS (`docs/PRODUCT.md` §2).

## Build from source

You need:

- Rust, the version pinned in `rust-toolchain.toml` (rustup installs it, with rustfmt and clippy),
  and cargo-deny (`cargo install cargo-deny --locked`).
- Node.js LTS and npm.
- Windows: the Visual Studio C++ Build Tools and the WebView2 runtime (part of Windows 11; Windows
  10 gets it with Microsoft Edge).
- Linux: the Tauri prerequisites, on Debian and Ubuntu
  `sudo apt-get install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`.

The full check builds the front end, then runs the format check, clippy, the typecheck, lint,
the licence gate and every test. It is what continuous integration runs:

```sh
sh scripts/verify.sh
```

PowerShell: `powershell -File scripts/verify.ps1`.

The tray app (the front end is embedded at compile time, so build it first):

```sh
npm --prefix app run build
cargo run -p redpen-app
```

`cargo run -p redpen-app -- --smoke` starts it, waits for the hidden interface window to report
ready, and exits 0: the launch check continuous integration runs.

The command-line tool:

```sh
cargo run -q -p redpen-cli -- --version --json
```

prints `{"name":"redpen","version":"0.1.0"}`.

## Repository

| Path | What |
|---|---|
| `crates/redpen-core` | The platform-free core: checking, rewriting, plagiarism matching, providers, storage. The clock and the network are injected, so tests run with fakes |
| `crates/redpen-os` | Reading and replacing a selection on Windows and Linux |
| `crates/redpen-cli` | The `redpen` command-line tool the golden tests drive, and the hygiene and recordings checks |
| `app/` | The React and TypeScript interface; `app/src-tauri/` is the Tauri shell (tray, windows, commands) |
| `scripts/` | `verify.sh` and `verify.ps1`, the icon generator |
| `tests/cases/` | Golden cases run through the `redpen` binary, with their recorded answers |
| `docs/` | Product, architecture, Publik integration and contract, testing, roadmap, one spec per module, decision records, the handbook |

## Documentation

- `docs/PRODUCT.md`: who it is for, every feature, every screen.
- `docs/ARCHITECTURE.md`: stack, layout, data model, commands, invariants, decisions.
- `docs/PUBLIK-INTEGRATION.md` and `docs/PUBLIK-CONTRACT.md`: every AI call, its prompt, schema,
  cost and errors.
- `docs/TESTING.md`: test layers, commands, fixtures, quality gates, the manual checklist.
- `docs/ROADMAP.md` and `docs/modules/`: the modules, in order, with their specifications.
- `docs/adr/`: decision records.
- `docs/HANDBOOK.md`: the user guide, grown with the app.
- `CONTRIBUTING.md`, `SECURITY.md`, `RELEASING.md`, `THIRD_PARTY.md`.

## Licence

Licensed under either the MIT licence (`LICENSE-MIT`) or the Apache License, Version 2.0
(`LICENSE-APACHE`), at your option (ADR-0002). `THIRD_PARTY.md` lists every dependency with its
licence.
