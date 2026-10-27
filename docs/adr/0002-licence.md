# ADR-0002: Licence: MIT OR Apache-2.0

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

Redpen is open source from the first commit. Its core (a Harper wrapper with spans in three
encodings, redaction, a minimal-edit guard, an OpenAI-compatible client with a cache and a status
sink, a snippet matcher and citation formatter) is the kind of code other projects may want to
reuse, and the Rust ecosystem's convention is dual MIT OR Apache-2.0, which keeps the code
compatible with both permissive and patent-granting uses. Every dependency the stack needs is
permissive, and the licence gate (`deny.toml`) keeps copyleft code out of the build.

## Decision

The code is licensed under MIT OR Apache-2.0, at the user's option (`LICENSE-MIT`,
`LICENSE-APACHE`), with the copyright line "Copyright (c) 2026 The Redpen contributors". The
licence gate allows only MIT, Apache-2.0 (with and without the LLVM exception), BSD-2-Clause,
BSD-3-Clause, ISC, Zlib, MPL-2.0, Unicode-3.0, Unicode-DFS-2016, BSL-1.0, CC0-1.0 and Unlicense;
everything else fails the build, transitive dependencies included.

## Consequences

- Anyone can reuse the core in a permissive or proprietary project; contributions are accepted
  under the same terms.
- The clean-room rule in `CONTRIBUTING.md` applies: the source of GPL, AGPL, SSPL, BUSL or
  source-available projects, GSpot (AGPL-3.0) included, is never open while writing the
  corresponding part of Redpen. Harper (Apache-2.0) may be studied.
- No copyleft dependency can ever be added, whatever it would save.

## Alternatives considered

- **GPL-3.0 or AGPL-3.0**: would bar permissive reuse of the core and would have let the project
  study GSpot directly; reuse of the core was judged more valuable.
- **MIT alone**: no explicit patent grant; the dual licence costs nothing and is the convention.
