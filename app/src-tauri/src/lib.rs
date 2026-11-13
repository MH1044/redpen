//! Redpen's desktop shell: a thin Tauri layer that owns the tray, the windows and the hotkeys,
//! and turns commands into calls into `redpen-core` (docs/ARCHITECTURE.md §3). The logic worth
//! testing here is the publik API module (`publik.rs`, with `status.rs` and `resolver.rs`), the
//! hotkey rules (`hotkeys.rs`) and the settings commands, all written Tauri-free so tests drive
//! them with fakes. The web view never holds a key and never touches the filesystem on its own.
//!
//! - Module 00: the tray icon and `--smoke`, which exits 0 as soon as the interface reports
//!   ready, or 1 after twenty seconds.
//! - Module 03: the publik commands, the status in `status.json` and the tray's balance line,
//!   `GET /wallet` at start only while publik API is chosen and set up, and the built-app
//!   self-test that `main` runs first ([`startup_self_test`]).
//! - Module 06: the check shortcut (`tauri-plugin-global-shortcut`, registered from Rust only),
//!   the popup, Settings and welcome windows ([`windows`]), the session and settings commands,
//!   and the tray's Open Settings and Pause Redpen. `--measure-popup` times the shortcut's path
//!   on Windows ([`measure`]).

pub mod commands;
pub mod hotkeys;
pub mod measure;
pub mod proof;
pub mod publik;
pub mod resolver;
pub mod status;
pub mod tray;
pub mod windows;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use redpen_core::clock::{Clock, Sleeper, SystemClock, TokioSleeper};
use redpen_core::flow::Sessions;
use redpen_core::net::{
    HttpRequest, HttpResponse, HttpTransport, ReqwestTransport, TransportError,
};
use redpen_core::platform::Capture;
use redpen_core::provider::{AiClient, PrivacyOptions};
use redpen_core::store::{Db, Settings};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, Wry};

use commands::CommandError;
use commands::settings::SettingsService;
use publik::{PublikOptions, PublikService, SharedSettings};
use resolver::RedpenResolver;
use status::{PublikStatusSink, StatusStore};

/// How long `--smoke` waits for the interface before giving up.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(20);

/// The event the interface hears when the publik status changes (docs/ARCHITECTURE.md §5).
pub const STATUS_EVENT: &str = "status://publik";

/// How the app was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The tray app.
    Normal,
    /// `--smoke`: exit 0 once the interface reports ready. No welcome window, nothing saved.
    Smoke,
}

/// Whether the app runs as a launch smoke test.
struct Smoke(bool);

/// What the commands share.
pub struct AppState {
    /// The publik commands.
    pub publik: Arc<PublikService>,
    /// The provider for chat calls.
    pub resolver: Arc<RedpenResolver>,
    /// The one chat client, its status sink reading publik API's headers.
    pub ai: Arc<AiClient>,
    /// The popup's sessions: the captured texts, in memory only.
    pub sessions: Arc<Sessions>,
    /// Settings, the hotkeys and the own key.
    pub settings: Arc<SettingsService>,
    /// The clock (this month's usage).
    pub clock: Arc<dyn Clock>,
    /// A press of the shortcut is still capturing.
    pub(crate) capturing: AtomicBool,
}

/// The tray, kept so the pause state and the balance line can change.
struct TrayState(Arc<tray::Tray<Wry>>);

/// Called by the interface once it has rendered (`app/src/ipc.ts`).
#[tauri::command]
fn ui_ready(app: AppHandle, smoke: State<'_, Smoke>) {
    if smoke.0 {
        println!("redpen-app --smoke: the interface reported ready");
        app.exit(0);
    }
}

// ---------------------------------------------------------------------------------------------
// Start-up.

/// The production transport (reqwest with rustls), built on first use so a start that sends
/// nothing never loads the TLS set-up.
pub struct ProductionTransport {
    inner: OnceLock<Result<ReqwestTransport, TransportError>>,
}

/// The transport the app and the self-test use.
pub fn production_transport() -> ProductionTransport {
    ProductionTransport {
        inner: OnceLock::new(),
    }
}

#[async_trait::async_trait]
impl HttpTransport for ProductionTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        match self.inner.get_or_init(ReqwestTransport::new) {
            Ok(t) => t.send(req).await,
            Err(e) => Err(e.clone()),
        }
    }
}

/// The built-app self-test hook, the first thing `main` runs (contract §3, test 18): `None`
/// unless `flag` (`PUBLIK_SELF_TEST`) is set to something; otherwise runs
/// [`publik::self_test`] on a current-thread runtime and returns its one line. Nothing else is
/// started: no window, no tray, no hotkey.
pub fn startup_self_test(flag: Option<OsString>, t: &dyn HttpTransport) -> Option<String> {
    let flag = flag?;
    if flag.is_empty() {
        return None;
    }
    // A thread of its own, so the hook works whether or not the caller is inside a runtime.
    let line = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map(|rt| rt.block_on(publik::self_test(t, publik::APP_VERSION)))
                    .ok()
            })
            .join()
            .ok()
            .flatten()
    });
    Some(line.unwrap_or_else(|| "publik self-test: failed: the self-test could not run.".into()))
}

/// The folder name under the operating system's config and data folders.
fn app_folder() -> &'static str {
    if cfg!(windows) { "Redpen" } else { "redpen" }
}

/// The config and data directories (docs/ARCHITECTURE.md §4).
fn app_dirs(app: &AppHandle) -> (PathBuf, PathBuf) {
    let path = app.path();
    let fallback = || PathBuf::from(".");
    (
        path.config_dir()
            .unwrap_or_else(|_| fallback())
            .join(app_folder()),
        path.local_data_dir()
            .unwrap_or_else(|_| fallback())
            .join(app_folder()),
    )
}

/// The capture of this operating system: Windows (module 05); none elsewhere yet, where the
/// shortcut says so instead of capturing ([`commands::session::CAPTURE_UNAVAILABLE`]).
pub fn system_capture() -> Arc<dyn Capture> {
    #[cfg(windows)]
    {
        Arc::new(redpen_os::WindowsCapture::new())
    }
    #[cfg(not(windows))]
    {
        Arc::new(NoCapture)
    }
}

/// The capture where there is none yet. It is never reached: no session opens without a
/// capture, and Accept, Copy and Undo need one.
#[cfg(not(windows))]
struct NoCapture;

#[cfg(not(windows))]
impl Capture for NoCapture {
    fn foreground(
        &self,
    ) -> Result<redpen_core::platform::Foreground, redpen_core::platform::CaptureError> {
        Err(redpen_core::platform::CaptureError::NothingCopied)
    }
    fn inspect(
        &self,
        _w: &redpen_core::platform::WindowRef,
    ) -> Result<Option<redpen_core::platform::Foreground>, redpen_core::platform::CaptureError>
    {
        Ok(None)
    }
    fn modifiers_down(&self) -> bool {
        false
    }
    fn read_selection(
        &self,
        _timeout: Duration,
    ) -> Result<String, redpen_core::platform::CaptureError> {
        Err(redpen_core::platform::CaptureError::NothingCopied)
    }
    fn focus(
        &self,
        _w: &redpen_core::platform::WindowRef,
    ) -> Result<bool, redpen_core::platform::CaptureError> {
        Ok(false)
    }
    fn paste(
        &self,
        _text: &str,
        _restore_delay: Duration,
    ) -> Result<(), redpen_core::platform::CaptureError> {
        Err(redpen_core::platform::CaptureError::WindowGone)
    }
    fn send_undo(&self) -> Result<(), redpen_core::platform::CaptureError> {
        Err(redpen_core::platform::CaptureError::WindowGone)
    }
    fn set_clipboard_text(&self, _text: &str) -> Result<(), redpen_core::platform::CaptureError> {
        Err(redpen_core::platform::CaptureError::ClipboardBusy)
    }
}

/// Builds the shared state over the production transport, with `registrar` for the hotkeys.
fn app_state(
    config_dir: &Path,
    data_dir: &Path,
    registrar: Arc<dyn hotkeys::HotkeyRegistrar>,
) -> AppState {
    let transport: Arc<dyn HttpTransport> = Arc::new(production_transport());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let sleeper: Arc<dyn Sleeper> = Arc::new(TokioSleeper);
    let loaded = Settings::load(config_dir).unwrap_or_else(|_| {
        tracing::warn!("settings.json could not be read; the defaults are used");
        Settings::default()
    });
    let privacy = PrivacyOptions::from(&loaded.privacy);
    let settings: SharedSettings = Arc::new(RwLock::new(loaded));
    let options = PublikOptions::current();
    // The last run's status only while publik API is still set up on this computer.
    let status = Arc::new(if options.resolve(None).credential().is_some() {
        StatusStore::load(data_dir, clock.clone())
    } else {
        StatusStore::new(data_dir, clock.clone())
    });
    let db = Db::open(data_dir).unwrap_or_else(|_| {
        tracing::warn!("redpen.db could not be opened; answers are kept in memory");
        Db::open_in_memory().expect("an in-memory database opens")
    });
    let ai = Arc::new(AiClient::new(
        transport.clone(),
        db,
        clock.clone(),
        sleeper.clone(),
        Arc::new(PublikStatusSink::new(status.clone())),
    ));
    ai.set_privacy(privacy);
    let resolver = Arc::new(RedpenResolver::new(
        settings.clone(),
        config_dir.to_path_buf(),
        options.clone(),
        status.clone(),
    ));
    let sessions = Arc::new(Sessions::new(
        system_capture(),
        ai.clone(),
        resolver.clone(),
        clock.clone(),
        sleeper,
    ));
    let settings_service = SettingsService::new(
        settings.clone(),
        config_dir,
        registrar,
        Arc::from(hotkeys::system_layout()),
    )
    .with_ai(ai.clone());
    let publik = PublikService::new(
        transport,
        options,
        config_dir.to_path_buf(),
        settings,
        status,
    );
    AppState {
        publik: Arc::new(publik),
        resolver,
        ai,
        sessions,
        settings: Arc::new(settings_service),
        clock,
        capturing: AtomicBool::new(false),
    }
}

/// Builds Harper for `settings` in the background, so the next popup is quick.
pub fn warm_checker(state: &AppState, settings: Settings) {
    let sessions = state.sessions.clone();
    tauri::async_runtime::spawn_blocking(move || sessions.warm(&settings));
}

/// Pause Redpen (`paused`) or resume: the hotkeys, `ui.paused` and the tray.
pub fn set_paused(app: &AppHandle, paused: bool) -> Result<Settings, CommandError> {
    let state = app.state::<AppState>();
    let result = state.settings.set_paused(paused);
    show_paused(app, state.settings.get().ui.paused);
    result
}

/// The tray's pause tick and icon for `paused`.
pub fn show_paused(app: &AppHandle, paused: bool) {
    if let Some(tray) = app.try_state::<TrayState>() {
        tray.0.show_paused(paused);
    }
}

/// What a tray menu item does.
fn on_tray_item(app: &AppHandle, id: &str) {
    match id {
        tray::SETTINGS_ID => {
            let target = windows::SettingsTarget::new("general", None);
            if windows::open_settings(app, &target).is_err() {
                tracing::warn!("Settings could not be opened");
            }
        }
        tray::PAUSE_ID => {
            let paused = app.state::<AppState>().settings.get().ui.paused;
            if let Err(e) = set_paused(app, !paused) {
                tracing::warn!(error = e.kind().unwrap_or("unknown"), "pause failed");
            }
        }
        tray::QUIT_ID => app.exit(0),
        _ => {}
    }
}

/// Builds and runs the app. The process ends when Quit is chosen or, in [`Mode::Smoke`], when
/// the interface reports ready.
pub fn run(mode: Mode) {
    let smoke = mode == Mode::Smoke;
    let shortcuts = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                commands::session::on_check_shortcut(app);
            }
        })
        .build();
    let app = tauri::Builder::default()
        .plugin(shortcuts)
        .manage(Smoke(smoke))
        .invoke_handler(tauri::generate_handler![
            ui_ready,
            commands::publik::consent_info,
            commands::publik::publik_connect,
            commands::publik::publik_disconnect,
            commands::publik::publik_refresh,
            commands::publik::provider_status,
            commands::publik::open_account_link,
            commands::publik::set_providers,
            commands::session::session_get,
            commands::session::apply_fix,
            commands::session::rewrite,
            commands::session::accept,
            commands::session::copy_result,
            commands::session::undo_last_replace,
            commands::session::close_session,
            commands::settings::settings_get,
            commands::settings::settings_update,
            commands::settings::own_key_save,
            commands::settings::own_key_clear,
            commands::settings::provider_test,
            commands::settings::local_models,
            commands::settings::usage_month,
            commands::settings::sent_log_list,
            commands::settings::sent_log_clear,
            commands::settings::cache_clear,
            commands::settings::app_pause,
            commands::settings::open_settings,
            commands::settings::welcome_done,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let (config_dir, data_dir) = app_dirs(&handle);
            let registrar = Arc::new(hotkeys::PluginRegistrar::new(handle.clone()));
            // Managed before any window opens, so the first command a window sends finds it.
            app.manage(app_state(&config_dir, &data_dir, registrar));
            let state = app.state::<AppState>();
            let settings = state.settings.get();

            windows::create_popup(&handle)?;
            let status = state.publik.status().clone();
            let tray = Arc::new(tray::Tray::build(
                &handle,
                tray::balance_label(status.current().as_ref()),
                settings.ui.paused,
                on_tray_item,
            )?);
            let listener_tray = tray.clone();
            let listener_handle = handle.clone();
            status.subscribe(Box::new(move |current| {
                listener_tray.show_balance(&listener_handle, tray::balance_label(current));
                if listener_handle.emit(STATUS_EVENT, current).is_err() {
                    tracing::warn!("the publik status event could not be sent");
                }
            }));
            app.manage(TrayState(tray));

            // The check shortcut, unless Redpen was paused when it last ran. A refusal (another
            // app holds the combination) is shown in Settings → General.
            if let Err(e) = state.settings.start() {
                tracing::info!(error = e.kind(), "the check shortcut was not registered");
            }
            let publik = state.publik.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(Err(e)) = publik.refresh_at_start().await {
                    tracing::info!(error = e.kind(), "the start-up wallet refresh failed");
                }
            });
            let ai = state.ai.clone();
            tauri::async_runtime::spawn(async move { ai.keep_sweeping().await });
            warm_checker(&state, settings.clone());
            let welcome = !smoke && windows::shows_welcome(&settings);
            if welcome {
                windows::open_welcome(&handle)?;
                if state.settings.mark_welcomed().is_err() {
                    tracing::warn!("the welcome window could not be marked as shown");
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::net::FakeTransport;

    #[test]
    fn the_self_test_never_runs_without_the_flag() {
        let fake = FakeTransport::scripted(Vec::new());
        assert_eq!(startup_self_test(None, &fake), None);
        assert_eq!(startup_self_test(Some(OsString::new()), &fake), None);
        assert!(fake.requests_seen().is_empty());
    }

    #[test]
    fn the_folders_are_named_as_the_architecture_says() {
        assert_eq!(
            app_folder(),
            if cfg!(windows) { "Redpen" } else { "redpen" }
        );
    }

    #[test]
    fn the_production_transport_loads_nothing_until_a_request() {
        let t = production_transport();
        assert!(t.inner.get().is_none());
    }
}
