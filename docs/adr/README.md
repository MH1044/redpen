# Decision records

Architecture decision records (ADRs): the decisions that are expensive to make and expensive to
reverse, each with what was rejected and why. `docs/ARCHITECTURE.md` §9 summarises them.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-stack.md) | Stack: Tauri 2, Rust core with harper-core, React interface | Accepted |
| [0002](0002-licence.md) | Licence: MIT OR Apache-2.0 | Accepted |
| [0003](0003-capture.md) | Capture: the clipboard route with UI Automation checks | Accepted |
| [0004](0004-prompts-and-the-guard.md) | Prompts and the guard: versioned prompt files, a minimal-edit guard for Fix grammar | Accepted |
| [0005](0005-publik-module.md) | Publik module: the reference module in the app crate, one AI client | Accepted |
| [0006](0006-no-streaming.md) | No streaming in v1: answers are shown after validation | Accepted |
| [0007](0007-name-and-slug.md) | Name and slug: Redpen, `redpen` | Proposed (maintainer decides at module 03, before listing) |
| [0008](0008-platforms.md) | Platforms: Windows and Linux, no macOS build | Accepted |
| [0009](0009-plagiarism-from-snippets.md) | Plagiarism from snippets: exact-phrase search, result pages never fetched | Accepted |

## Rules

- Append-only. An accepted record is never edited; a change gets a new record that supersedes it,
  and the old one's status becomes `Superseded by ADR-NNNN`.
- Only decisions that are expensive to make and expensive to reverse. Anything else goes in the
  module spec's Design notes.
- Record what was rejected and the deciding factor for each. That section is the one people come
  back to.
- A `Proposed` record names who decides and by when. The module that depends on it waits until it
  is `Accepted`.

To add one, copy `template.md` to `NNNN-short-title.md` with the next number, fill every section,
and add a row above.
