# ADR-0006: No streaming in v1: answers are shown after validation

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

Every rewrite answer is a JSON object that must pass its schema, the marker checks (no private
detail dropped or invented), the output cleaning and the guard before it can be shown or inserted
(invariant 8). A paragraph rewrite arrives in one to three seconds on `publik-balanced`. The
Chat Completions dialect can stream tokens, which would let the popup show text as it arrives.

## Decision

Rewrites are not streamed in v1. The popup shows "Rewriting… (publik-balanced)" with Cancel, and
the result appears whole, after validation. Cancel stops waiting; the call is not retried.

## Consequences

- Nothing unvalidated is ever on screen: a user cannot read, copy or accept half an answer that
  then fails a marker check.
- The parsing and validation code has one input shape (a complete body), and the fake transport
  needs no streaming mode.
- Costs: a wait of one to three seconds with no visible progress, which the loading line and
  Cancel soften; a 1,500-word rewrite waits longer.

## Alternatives considered

- **Streaming partial JSON**: shows unvalidated text, and a JSON object cannot be validated before
  it is complete; the diff and the guard need the whole answer anyway.
- **Streaming behind a "preview" label**: still shows text that may be refused a second later, and
  invites copying it.
