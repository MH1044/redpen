//! Redpen's desktop shell: a thin Tauri layer that owns the tray, the windows and the hotkeys,
//! and turns commands into calls into `redpen-core` (docs/ARCHITECTURE.md §3). It holds no
//! logic worth testing apart from the Publik module that arrives with module 03. The web view
//! never holds a key and never touches the filesystem on its own.
//!
//! Module 00: a tray icon with **Quit** and one hidden interface window. With `--smoke` the app
//! exits 0 as soon as that window reports ready, or 1 after twenty seconds.

use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, RunEvent, State};

/// How long `--smoke` waits for the interface before giving up.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(20);

/// Whether the app runs as a launch smoke test.
struct Smoke(bool);

/// Called by the interface once it has rendered (`app/src/ipc.ts`).
#[tauri::command]
fn ui_ready(app: AppHandle, smoke: State<'_, Smoke>) {
    if smoke.0 {
        println!("redpen-app --smoke: the interface reported ready");
        app.exit(0);
    }
}

/// Builds and runs the app. The process ends when Quit is chosen or, with `smoke`, when the
/// interface reports ready.
pub fn run(smoke: bool) {
    let app = tauri::Builder::default()
        .manage(Smoke(smoke))
        .invoke_handler(tauri::generate_handler![ui_ready])
        .setup(move |app| {
            build_tray(app.handle())?;
            if smoke {
                arm_smoke_timeout();
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Redpen could not start");

    app.run(|_app, event| {
        // A tray app keeps running when its windows close; only Quit (or the smoke test) ends it.
        if let RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            api.prevent_exit();
        }
    });
}

/// The tray icon with its menu: Quit only, until the Settings, balance and Pause items arrive.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&quit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("the app icon is embedded at build time");
    TrayIconBuilder::with_id("redpen")
        .icon(icon)
        .tooltip("Redpen")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "quit" {
                app.exit(0);
            }
        })
        .build(app)?;
    Ok(())
}

/// Ends a smoke run with exit code 1 if the interface never reports ready.
fn arm_smoke_timeout() {
    std::thread::spawn(|| {
        std::thread::sleep(SMOKE_TIMEOUT);
        eprintln!(
            "redpen-app --smoke: the interface did not report ready within {} s",
            SMOKE_TIMEOUT.as_secs()
        );
        std::process::exit(1);
    });
}
