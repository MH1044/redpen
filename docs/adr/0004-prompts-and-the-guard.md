# ADR-0004: Prompts and the guard: versioned prompt files, a minimal-edit guard for Fix grammar

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

Every rewrite is a prompt rendered from a template, sent to a model, and parsed back. The rendered
request is golden-tested (`docs/TESTING.md` §1), answers are cached by a key that includes the
prompt's identity, and a prompt that drifts silently would make the goldens lie and serve stale
cached answers for a changed prompt. Separately, models asked to fix grammar tend to rephrase:
a "Fix grammar" that rewrites half the paragraph is not what the user pressed, and Enter accepting
it blindly would replace their text with something they did not review.

## Decision

Prompts are files, `prompts/<id>.v<N>.md`, with front matter naming the tier, temperature, token
limit and response schema, loaded at build time and rendered with named placeholders. A prompt
file is never edited in place: any change is a new version, which also orphans the old version's
cached answers on purpose (`docs/PUBLIK-INTEGRATION.md` §6). Fix grammar runs at temperature 0
and its result passes a minimal-edit guard: word tokens are diffed, each hunk counts the larger of
its deleted and inserted tokens, and when the changed count is at least 4 and exceeds 35% of the
original's tokens the result is shown with the warning "This changes more than a grammar fix
should (9 of 14 words). Check it before accepting." and Enter no longer accepts it (the Accept
button still does). The thresholds live in one place and are confirmed or adjusted by the first
run of the `eval-rewrite` gate.

## Consequences

- The exact request for every action is a golden file, and a prompt change is visible as a new
  file in the diff.
- A cache never serves an answer written for a different prompt.
- Over-editing answers to Fix grammar are still shown, but never accepted by reflex.
- Costs: two files to touch for a prompt change (the new version and the action table), and a
  guard that may warn on a legitimately heavy fix, which the gate measures (warnings on at most 5%
  of correct fixes).

## Alternatives considered

- **Editing prompts in place**: the goldens would need regenerating with every edit, and cached
  answers for the old prompt would keep being served.
- **No guard**: models over-edit; the user pressed Fix grammar, not Clearer.
- **Rejecting over-edited answers outright**: the answer may still be right; showing it with a
  warning keeps the user in control.
