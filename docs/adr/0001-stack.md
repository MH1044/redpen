# ADR-0001: Stack: Tauri 2, Rust core with harper-core, React interface

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

Redpen is a resident tray app that must start with the OS, stay small, react to a global hotkey
within a few hundred milliseconds, read and replace a selection in any other app, and keep a
per-install API key out of the interface. The checking must be local, fast and private: Harper
(`harper-core`, Apache-2.0) is a Rust library that lints English in well under 50 ms for a
thousand words. The logic that matters (checking, rewriting, validation, redaction, the money
path, plagiarism matching) must be testable without a window or a display, on both CI runners.

## Decision

A Tauri 2 desktop app for Windows and Linux: the logic in a Rust library crate
(`crates/redpen-core`) with the platform capture code in `crates/redpen-os`, a thin Tauri shell
(`app/src-tauri`) that only owns the tray, the windows, the hotkeys and the commands, and a React
and TypeScript interface built with Vite for the popup and Settings. One stable Rust toolchain is
pinned in `rust-toolchain.toml`. The app identifier is `org.redpen.app` as a placeholder until a
domain is settled; it is fixed at the packaging module, because installers key on it.

## Consequences

- One language for the core and the capture code, with a test suite that runs on both CI runners
  without a display; the interface is unit-tested with Vitest and launch-tested end to end.
- The key never reaches the web view: the interface only sees a status object and keyless results.
- The installer stays small (the system web view is used), and the resident process is light.
- Costs: Tauri's Linux prerequisites, two toolchains (Rust and Node) for every contributor, a web
  view that differs slightly per platform, and a hidden window that must exist before the popup
  can open quickly.

## Alternatives considered

- **Electron**: too heavy for a resident tray app that must always be running; a bundled browser
  for a popup of a few hundred pixels.
- **A browser extension only**: not system-wide; the point of Redpen is Outlook, Teams, Word, Slack
  and every other app, not only web pages.
- **A native toolkit per platform**: two interfaces to keep equal, for a popup and a Settings
  window that a web view draws well enough.
