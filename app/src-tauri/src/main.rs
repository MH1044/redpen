//! `redpen-app`: the tray app. `--smoke` starts it, waits for the hidden interface window to
//! report ready, and exits 0 (docs/TESTING.md §1, the launch smoke). `--measure-popup` times the
//! check shortcut's path from the press to the findings on Windows and prints `{"p95_ms": N}`
//! (module 06).
//!
//! With `PUBLIK_SELF_TEST` set, the first lines run the publik built-app self-test instead
//! (docs/modules/03-publik.md): one line on standard output, then the process ends before any
//! window, the tray or a hotkey exists.

// A release build has no console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(line) = redpen_app::startup_self_test(
        std::env::var_os("PUBLIK_SELF_TEST"),
        &redpen_app::production_transport(),
    ) {
        attach_parent_console();
        println!("{line}");
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--measure-popup") {
        attach_parent_console();
        std::process::exit(redpen_app::measure::run());
    }
    let mode = if args.iter().any(|arg| arg == "--smoke") {
        redpen_app::Mode::Smoke
    } else {
        redpen_app::Mode::Normal
    };
    redpen_app::run(mode);
}

/// A release build on Windows starts without a console, so when its output is not redirected
/// the self-test line would go nowhere: it attaches to the console of the shell that started
/// it. Redirected output (a pipe or a file) is left as it is.
#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
    };
    // SAFETY: both calls take no pointers; a failure leaves the process as it was.
    unsafe {
        // A null or invalid handle means there is nowhere to write.
        let redirected = GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|h| !h.is_invalid());
        if !redirected {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

/// Debug builds and Linux have a console already.
#[cfg(not(all(windows, not(debug_assertions))))]
fn attach_parent_console() {}
