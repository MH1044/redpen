# ADR-0008: Platforms: Windows and Linux, no macOS build

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

GSpot (AGPL-3.0, listed and publik verified, re-checked on 26 October 2026) already is a
Grammarly-style assistant for every Mac app: Harper checks in place, AI rewrites, tone,
translation and a plagiarism check, through publik API or the user's own key. It has no Windows or
Linux build. The gap Redpen fills is on Windows and Linux (`docs/PRODUCT.md` §2), and capture on
macOS would need its own accessibility and secure-input code, its own manual checklist and a
signed, notarised build to be usable at all.

## Decision

Redpen ships for Windows 10 (1809) and 11 on x64, and for Linux (X11 fully; Wayland with its
limits stated in the app). There is no macOS build. CI runs on `windows-latest` and
`ubuntu-latest`; the licence gate checks the two targets; the manual checklist has no macOS list.

## Consequences

- The capture code, the installers and the manual testing cover two platforms instead of three,
  and the Windows path is built and proven first.
- Mac users are pointed at GSpot, which does the same job there and is also on Publik.
- A macOS build stays possible later: the core is platform-free, and `docs/PRODUCT.md` §8 keeps
  it as a "later" item in case GSpot stops being maintained.

## Alternatives considered

- **A macOS build**: would remake GSpot, which already exists and is maintained, for the cost of a
  third capture implementation, notarisation and a third checklist.
- **Windows only**: Linux costs one capture implementation over X11 and a clipboard-only mode on
  Wayland, and the student on a Linux laptop is one of the three people Redpen is for.
