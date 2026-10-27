# ADR-0005: Publik module: the reference module in the app crate, one AI client

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

The publik API contract (`docs/PUBLIK-CONTRACT.md` §3) asks every app to copy the official
reference module for its stack verbatim and adapt only what it lists, so Publik's future changes
can be merged by diff. The reference Rust module shells out to `curl`; the contract allows an
in-process HTTP client on desktop as long as the key never goes on a command line. Redpen has one
consumer of the module (the Tauri shell) and three providers (publik API, the user's own key, a
local model) that all speak the same Chat Completions dialect.

## Decision

The reference `publik.rs` is copied into `app/src-tauri/src/publik.rs` at module 03, with only
the slug, the version and the adaptations A1 to A8 of `docs/PUBLIK-INTEGRATION.md` §2 changed,
each marked `// redpen:`. HTTP goes through `redpen_core::net::HttpTransport` (reqwest with
rustls in production, the fake transport in tests) for provisioning, the wallet, chat and search
alike, so the key only ever sits in an in-memory `Authorization` header. One OpenAI-compatible
client in the core, `AiClient`, serves every provider; the Publik module resolves the credential
and hands it `{base_url, key}`.

## Consequences

- A future reference update is a diff against a file with a handful of marked changes.
- One transport means one fake, so every Publik test and every provider test runs offline against
  the same `recording.json` format.
- The key never appears on a command line, in a log or in the web view (invariant 6).
- Costs: the tier rule and the search types live in the core (the core cannot depend on the app
  crate), so the module is split across two crates, and the reference's `curl` path is replaced
  rather than reused.

## Alternatives considered

- **A separate `redpen-publik` crate**: allowed by the contract and tidy, but with one consumer it
  adds a crate boundary for nothing; it can be split out later if a second consumer appears.
- **Keeping the reference's `curl` shell-out**: a second HTTP path to fake, and the key would have
  to be passed to a child process.
- **A different client per provider**: three implementations of one dialect, and three sets of
  error mappings to keep equal.
