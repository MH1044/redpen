# 10: Packaging and release

## Goal

A tester can install Redpen from a GitHub release on Windows or Linux, have it start
with the OS if they choose, and hear about new versions without the app sending anything but one
request. The maintainer cuts a release by pushing a tag; a workflow verifies, builds, drafts with
the notes, and publishes only when green. The Publik listing points at real installers.

## Depends on

All earlier modules.

## Scope

- Tauri bundles: Windows NSIS installer, per-user (no administrator), which bootstraps WebView2 on
  Windows 10; Linux AppImage and `.deb`. No macOS bundle (docs/PRODUCT.md §2).
- Start with the OS, opt-in (Settings → General "Start Redpen when I sign in"), through
  `tauri-plugin-autostart`.
- Update check: `GET https://api.github.com/repos/<account>/redpen/releases/latest` with only
  `User-Agent: Redpen/<version>` and `Accept: application/vnd.github+json`; at start and every
  24 hours; off in development builds and with Settings → General "Check for updates"; a newer
  `tag_name` (semver) adds the tray item "Redpen 0.5.0 is available", which opens the release page
  in the browser; nothing is downloaded; any failure is silent.
- `release.yml`: on a tag `v<version>`, run `verify` on both runners (`windows-latest`,
  `ubuntu-latest`), build the bundles, and
  refuse with a message when `docs/releases/<version>.md` is missing, when the versions in
  `Cargo.toml`, `app/src-tauri/tauri.conf.json` and `app/package.json` differ from the tag, or
  when `app/src-tauri/publik-app-token.txt` has no `pat_` token; draft the release with the notes,
  upload, publish when every job is green.
- An installer smoke job: silent install into a temporary folder, `--smoke`, silent uninstall.
- About screen: version, licence, and the third-party list generated at build time from
  `THIRD_PARTY.md`.
- A test that every URL host written in non-test source is listed in `SECURITY.md`.
- The listing draft of docs/PRODUCT.md §9 checked (summary at most 80 characters, no URL). The
  maintainer updates the Publik listing with `publish.md`; releases update it automatically.

## Files

- `app/src-tauri/tauri.conf.json` (bundle targets), `app/src-tauri/src/{updates.rs, autostart.rs}`
- `.github/workflows/release.yml`, `.github/workflows/ci.yml` (installer smoke)
- `scripts/release-check.{sh,ps1}`, `docs/releases/0.1.0.md`
- `RELEASING.md`, `README.md`, `docs/HANDBOOK.md`, `SECURITY.md`, `THIRD_PARTY.md`
- Tests: `app/src-tauri/tests/10_release.rs`, `crates/redpen-cli/tests/10_release_checks.rs`

## Interfaces

```rust
pub struct UpdateInfo { pub version: semver::Version, pub url: String }
pub async fn check_for_update(t: &dyn HttpTransport, current: &semver::Version,
                              enabled: bool, dev_build: bool) -> Option<UpdateInfo>;
```

Messages: tray "Redpen 0.5.0 is available"; release refusals "docs/releases/0.5.0.md is
missing.", "Version mismatch: tag 0.5.0, tauri.conf.json 0.4.2.", "Publik app token missing; see
docs/PUBLIK-INTEGRATION.md §2."

## Out of scope

- Code signing: it costs money, so the maintainer decides; until then `RELEASING.md` explains the
  warnings.
- Automatic updates (download and install).

## Design notes

- Decided: releases are cut by tags only; the workflow publishes nothing that failed on any runner.
- `RELEASING.md` covers tags, notes and what testers see: Windows SmartScreen ("Windows protected
  your PC": More info → Run anyway), Linux (make the AppImage executable, or install the `.deb`),
  and that some antivirus tools flag unsigned apps that send keystrokes. README gets the install section; the HANDBOOK is completed.
- Further decisions are filled in here during implementation.

## Acceptance tests

Code tests in `app/src-tauri/tests/10_release.rs` and `crates/redpen-cli/tests/10_release_checks.rs`;
CI jobs for installers.

1. `check_for_update` with current 0.4.0 and a fake answer `{"tag_name": "v0.5.0", "html_url":
   "https://github.com/example/redpen/releases/tag/v0.5.0"}` → `Some(0.5.0)`; `v0.4.0` → `None`;
   `v0.4.0-beta.1` → `None`; a malformed body → `None` and no error shown.
2. `enabled: false` or `dev_build: true` → no request.
3. The update request's headers are exactly `User-Agent` and `Accept`; no query string, cookie or
   authorisation.
4. The autostart toggle calls the fake plugin's enable and disable once each.
5. `scripts/release-check` with a missing notes file, a version mismatch and the placeholder token
   file → each refusal message, exit code 1.
6. `Cargo.toml`, `tauri.conf.json` and `app/package.json` carry the same version.
7. The listing summary in docs/PRODUCT.md §9 has at most 80 characters and no `http`.
8. Windows installer smoke: `/S /D=<temp>` installs, `redpen-app --smoke` exits 0, silent uninstall
   exits 0 and removes the folder.
9. The AppImage runs `--smoke` under `xvfb-run` and exits 0.
10. The About screen's list equals the rows parsed from `THIRD_PARTY.md`.
11. Every host in an `https://` or `http://` literal in `crates/*/src`, `app/src` and
    `app/src-tauri/src` (`publikhq.com`, `api.search.brave.com`, `api.github.com`, `localhost`, and
    any other) appears in
    `SECURITY.md`.

## Status

Planned
