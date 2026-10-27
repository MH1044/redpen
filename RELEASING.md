# Releasing

Releases are cut by pushing a tag. The Release workflow (`.github/workflows/release.yml`) refuses
to run without release notes, and, from the packaging module onwards (`docs/modules/10-packaging-and-release.md`),
verifies on both runners, builds the installers, drafts the GitHub release with the notes, uploads
the artifacts and publishes only when every job is green.

## One version, three places

The version lives in `Cargo.toml` under `[workspace.package]`, in `app/src-tauri/tauri.conf.json`
and in `app/package.json`. `redpen --version` and the About screen report it. The release check of
the packaging module refuses a tag whose version differs from any of the three.

## Steps

1. Write the notes: `docs/releases/<version>.md`, with what is new, what is known to be broken, and
   the result of the manual checklist in `docs/TESTING.md` §8 on Windows and Linux. The workflow's
   first job fails with `docs/releases/<version>.md is missing; write the release notes first.`
   when the file is absent.
2. Bump the version in the three places above.
3. `sh scripts/verify.sh` green, then commit: `chore(release): v<version>`.
4. Tag and push: `git tag v<version>`, `git push origin main`, `git push origin v<version>`.
5. Watch the Release run to completion. A red run is fixed by the next commit and a new tag; a tag
   is never moved.

## What a tester sees with an unsigned installer

Installers are unsigned until a signing certificate exists.

- **Windows**: SmartScreen shows "Windows protected your PC". Click **More info**, then
  **Run anyway**. The installer then runs normally. Some antivirus tools flag unsigned apps that
  send keystrokes; Redpen types into other apps on Accept, so a warning there is expected until
  the installer is signed.
- **Linux**: mark the AppImage executable (`chmod +x`) and run it, or install the `.deb` with the
  package manager.

There is no macOS build (ADR-0008). Testers are told this in the release notes of every unsigned
release.
