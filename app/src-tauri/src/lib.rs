//! Redpen's desktop shell: a thin Tauri layer that owns the tray, the windows and the hotkeys,
//! and turns commands into calls into `redpen-core` (docs/ARCHITECTURE.md §3). The logic worth
//! testing here is the publik API module (`publik.rs`, with `status.rs` and `resolver.rs`),
//! written Tauri-free so tests drive it with the fake transport. The web view never holds a key
//! and never touches the filesystem on its own.
//!
//! - Module 00: a tray icon with **Quit** and one hidden interface window. With `--smoke` the app
//!   exits 0 as soon as that window reports ready, or 1 after twenty seconds.
//! - Module 03: the publik commands, the status in `status.json` and the tray's balance line,
//!   `GET /wallet` at start only while publik API is chosen and set up, and the built-app
//!   self-test that `main` runs first ([`startup_self_test`]).

pub mod commands;
pub mod proof;
pub mod publik;
pub mod resolver;
pub mod status;
pub mod tray;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use redpen_core::clock::{Clock, SystemClock, TokioSleeper};
use redpen_core::net::{
    HttpRequest, HttpResponse, HttpTransport, ReqwestTransport, TransportError,
};
use redpen_core::provider::{AiClient, PrivacyOptions};
use redpen_core::store::{Db, Settings};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

use publik::{PublikOptions, PublikService, SharedSettings};
use resolver::RedpenResolver;
use status::{PublikStatusSink, StatusStore};

/// How long `--smoke` waits for the interface before giving up.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(20);

/// The event the interface hears when the publik status changes (docs/ARCHITECTURE.md §5).
pub const STATUS_EVENT: &str = "status://publik";

/// Whether the app runs as a launch smoke test.
struct Smoke(bool);

/// What the commands share.
pub struct AppState {
    /// The publik commands.
    pub publik: Arc<PublikService>,
    /// The provider for chat calls (the rewrite flow, from module 04).
    pub resolver: Arc<RedpenResolver>,
    /// The one chat client, its status sink reading publik API's headers.
    pub ai: Arc<AiClient>,
}

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

/// Builds the shared state over the production transport.
fn app_state(config_dir: &Path, data_dir: &Path) -> AppState {
    let transport: Arc<dyn HttpTransport> = Arc::new(production_transport());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
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
    let ai = AiClient::new(
        transport.clone(),
        db,
        clock,
        Arc::new(TokioSleeper),
        Arc::new(PublikStatusSink::new(status.clone())),
    );
    ai.set_privacy(privacy);
    let resolver = RedpenResolver::new(
        settings.clone(),
        config_dir.to_path_buf(),
        options.clone(),
        status.clone(),
    );
    let publik = PublikService::new(
        transport,
        options,
        config_dir.to_path_buf(),
        settings,
        status,
    );
    AppState {
        publik: Arc::new(publik),
        resolver: Arc::new(resolver),
        ai: Arc::new(ai),
    }
}

/// Builds and runs the app. The process ends when Quit is chosen or, with `smoke`, when the
/// interface reports ready.
pub fn run(smoke: bool) {
    let app = tauri::Builder::default()
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
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let (config_dir, data_dir) = app_dirs(&handle);
            let state = app_state(&config_dir, &data_dir);
            let status = state.publik.status().clone();
            let tray = Arc::new(tray::Tray::build(
                &handle,
                tray::balance_label(status.current().as_ref()),
            )?);
            let listener_handle = handle.clone();
            status.subscribe(Box::new(move |current| {
                tray.show_balance(&listener_handle, tray::balance_label(current));
                if listener_handle.emit(STATUS_EVENT, current).is_err() {
                    tracing::warn!("the publik status event could not be sent");
                }
            }));
            let publik = state.publik.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(Err(e)) = publik.refresh_at_start().await {
                    tracing::info!(error = e.kind(), "the start-up wallet refresh failed");
                }
            });
            let ai = state.ai.clone();
            tauri::async_runtime::spawn(async move { ai.keep_sweeping().await });
            app.manage(state);
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
