//! `redpen-app`: the tray app. `--smoke` starts it, waits for the hidden interface window to
//! report ready, and exits 0 (docs/TESTING.md §1, the launch smoke).

// A release build has no console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let smoke = std::env::args().skip(1).any(|arg| arg == "--smoke");
    redpen_app::run(smoke);
}
