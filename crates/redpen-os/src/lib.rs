//! Redpen's platform code: the `Capture` implementations that read a selection from the app in
//! front and type a result back (docs/ARCHITECTURE.md §3). The trait, its refusals and its fake
//! are in `redpen_core::platform`.
//!
//! Windows ([`windows`], module 05) goes through the clipboard with UI Automation and token
//! checks first (ADR-0003). Linux arrives with module 07; until then this crate has nothing on
//! Linux. There is no macOS build (ADR-0008).

#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub use crate::windows::WindowsCapture;
