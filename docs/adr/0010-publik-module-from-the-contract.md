# ADR-0010: Publik module written from the contract, without a sign-in gate or usage telemetry

- **Status:** Accepted
- **Date:** 2026-11-13
- **Supersedes:** ADR-0005

## Context

ADR-0005 planned to copy Publik's official Rust reference module (`https://publikhq.com/sdk/publik.rs.md`)
into `app/src-tauri/src/publik.rs` and mark every change. Fetched again before module 03, on
2026-11-12, the reference had changed in ways the plan did not foresee:

- it states no licence, so its code cannot be copied into a repository licensed MIT OR Apache-2.0
  (ADR-0002, `CONTRIBUTING.md`);
- it opens every app with a "Sign in with publik" gate before the first window, with no other way
  past it, then a screen asking how to pay for AI;
- after the gate it sends usage telemetry to `POST /telemetry` (`app_opened` at start, then
  `active_session` every 30 minutes), which only the `DO_NOT_TRACK` or `PUBLIK_TELEMETRY`
  environment variables switch off;
- it sends the computer's hostname as `device_name`;
- it no longer carries the `DISCLOSURE` text or `balance_line` that `docs/PUBLIK-CONTRACT.md`
  describes.

Publik's listing pages say that it counts no users for an app without its module and holds a
fellowship listing without it.

Redpen's documents promise the opposite on each point: a check within a minute of first launch with
no account (`docs/PRODUCT.md` §7), consent at the first AI action and never at launch
(`docs/PUBLIK-INTEGRATION.md` §4), nothing leaving the computer without an explicit action
(`docs/ARCHITECTURE.md` §7, invariant 1), no telemetry (`SECURITY.md`), and a generic device label
instead of the hostname.

## Decision

`app/src-tauri/src/publik.rs` is written for Redpen from `docs/PUBLIK-CONTRACT.md`, the HTTP
contract every Publik app implements, and not copied from the reference. It keeps the contract's
public names (`DISCLOSURE`, `resolve_provider`, `provision`, `credential_path`, `balance_line`,
`top_up_url_from_402`, `self_test`, `search`) and the adaptations A1 to A8 of
`docs/PUBLIK-INTEGRATION.md` §2 as design rules. It has no start-up gate and sends no usage
telemetry: Publik is set up only after the user chooses it on the consent screen at the first AI
action, and `device_name` is `Windows PC` or `Linux PC`. `DISCLOSURE` and `balance_line` are
Redpen's own text, written from the contract's privacy and money sections. The rest of ADR-0005
stands: the module lives in the app crate, all HTTP goes through `redpen_core::net::HttpTransport`
(reqwest in production, the fake transport in tests), and one AI client serves every provider.

## Consequences

- The privacy promises in `docs/PRODUCT.md`, `SECURITY.md` and the invariants hold unchanged, and no
  code of unknown licence enters the repository.
- Publik counts no Redpen users, and may hold or annotate the listing. Whether to add a sign-in
  step or usage counting is the maintainer's decision when the app is listed; either would be a
  new record superseding this one.
- Publik's future changes cannot be merged from the reference by diff; module 03 and later
  modules fetch the contract's sources again before touching the money path, as before.
- If Publik publishes the reference under a licence compatible with MIT OR Apache-2.0, adopting
  it becomes possible again.

## Alternatives considered

- **Copy the reference as ADR-0005 planned**: no licence, a mandatory sign-in before a free local
  checker, and telemetry the project's rules forbid.
- **Copy only the parts Redpen needs**: still code of unknown licence.
- **A sign-in step at the first publik API action, with usage counting while publik API is the
  chosen provider behind a Settings switch**: keeps the checker free of any account and gives
  Publik its counts; left to the maintainer to decide at listing, because it changes what leaves
  the computer.
