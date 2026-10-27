# ADR-0007: Name and slug: Redpen, `redpen`

- **Status:** Proposed (the maintainer decides at module 03, before listing)
- **Date:** 2026-10-27

## Context

The app is listed on Publik under a slug, and the app token that mints per-install keys belongs to
that slug (`docs/PUBLIK-CONTRACT.md` §2). The slug also names the credential file on every user's
computer, the config and data directories, and the binaries. Changing it after the listing exists
or after users have installed the app means a new listing and a migration of every directory.
Whether the name Redpen and the slug `redpen` are free on Publik is confirmed only when the listing
is created.

## Decision

Proposed: the product is called Redpen and its slug is `redpen`; the binaries are `redpen` and
`redpen-app`, the directories `Redpen` (Windows) and `redpen` (Linux), and the Publik credential
`redpen.json`. The maintainer confirms the name is free, or chooses another, at module 03, before
the listing is created. Until then the names stay as planned and the live Publik path waits.

## Consequences

- Module 03 is completed and tested against the fake server with the slug `redpen`; only the
  listing and the token wait for the decision.
- If the name changes, the slug, the directory names, the credential file name and every string
  that shows the name change in one commit, before anything is listed or released.

## Alternatives considered

None yet. The record exists so that the name is decided once, before it is expensive to change.
