//! Settings commands (docs/ARCHITECTURE.md §5): `settings_get`, `settings_update`,
//! `own_key_save`, `own_key_clear`, `provider_test`, `local_models`, `usage_month`,
//! `sent_log_list`, `sent_log_clear`, `cache_clear` and `app_pause`.
//!
//! The rules are in [`SettingsService`], without Tauri, so tests drive it with a fake hotkey
//! registrar and keyboard layout: a patch is merged into the settings and checked; a new check
//! shortcut goes through [`Hotkeys::change`] (refused with `HotkeyInUse` or the AltGr message,
//! the old one still registered); `settings.json` is written last, and a failed write undoes the
//! shortcut change. No result carries a key: the own key goes into `secrets.json` and comes back
//! only as `own_key_saved: true`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use redpen_core::clock::Clock;
use redpen_core::net::url_host;
use redpen_core::provider::local::is_local_url;
use redpen_core::provider::{
    AiClient, LocalModel, PrivacyOptions, ProviderTest, ResolvedProvider, SecretString,
};
use redpen_core::store::{Secrets, SentEntry, Settings, local_day};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, State};

use super::CommandError;
use crate::AppState;
use crate::hotkeys::{HotkeyError, HotkeyRegistrar, Hotkeys, KeyboardLayout};
use crate::publik::SharedSettings;
use crate::windows::SettingsTarget;

/// The own key's address must be `https://`, or `http://` on this computer.
pub const INVALID_URL: &str =
    "Use an https:// address (or http://localhost for a server on this computer).";

/// `provider_test` for a kind with nothing to test.
pub const NOTHING_TO_TEST: &str = "There is nothing to test for that provider.";
/// `own_key_save` when `secrets.json` cannot be written.
pub const KEY_NOT_SAVED: &str = "Your key could not be saved.";
/// `own_key_clear` when `secrets.json` cannot be written.
pub const KEY_NOT_REMOVED: &str = "Your key could not be removed.";
/// `sent_log_clear` when the database fails.
pub const LOG_NOT_CLEARED: &str = "The log could not be cleared.";
/// `cache_clear` when the database fails.
pub const CACHE_NOT_CLEARED: &str = "The remembered answers could not be cleared.";
/// `open_settings` when the window cannot be made or shown.
pub const SETTINGS_NOT_OPENED: &str = "Settings could not be opened.";

/// Settings as the window receives them: every setting, whether an own key is saved (never the
/// key), whether the file is read-only (written by a newer Redpen), and why the check shortcut
/// is not working when the system refused it at start (another app held it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsView {
    #[serde(flatten)]
    pub settings: Settings,
    pub own_key_saved: bool,
    pub read_only: bool,
    pub hotkey_problem: Option<String>,
}

/// `own_key_save`'s argument. The key is moved into `secrets.json` and never sent back.
#[derive(Clone, Deserialize)]
pub struct OwnKeyInput {
    pub base_url: String,
    pub key: String,
    pub chat_model: String,
}

impl std::fmt::Debug for OwnKeyInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnKeyInput")
            .field("base_url", &self.base_url)
            .field("key", &"***")
            .field("chat_model", &self.chat_model)
            .finish()
    }
}

/// `usage_month`: this month's publik API use, from `usage_daily`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UsageMonth {
    pub total_micros: u64,
    pub calls: u64,
    pub by_feature: Vec<FeatureUsage>,
}

/// One feature's line of [`UsageMonth`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeatureUsage {
    pub feature: String,
    pub calls: u64,
    pub charge_micros: u64,
}

/// `{"kind": "Invalid", "message", "field"}` for a setting that cannot take a value.
pub fn invalid_field(field: &str) -> CommandError {
    CommandError(json!({
        "kind": "Invalid",
        "message": format!("That value can't be used for {field}."),
        "field": field,
    }))
}

impl From<HotkeyError> for CommandError {
    fn from(e: HotkeyError) -> Self {
        CommandError(e.to_json())
    }
}

/// Every leaf of `patch`, as a dotted path and its value (arrays are leaves).
fn leaves(prefix: &str, patch: &Value, out: &mut Vec<(String, Value)>) {
    match patch {
        Value::Object(map) => {
            for (k, v) in map {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                leaves(&path, v, out);
            }
        }
        other => out.push((prefix.to_string(), other.clone())),
    }
}

fn set_path(target: &mut Value, path: &str, value: Value) {
    let mut node = target;
    let parts: Vec<&str> = path.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        if !node.is_object() {
            *node = Value::Object(Default::default());
        }
        let map = node.as_object_mut().expect("an object");
        if i + 1 == parts.len() {
            map.insert((*part).to_string(), value);
            return;
        }
        node = map.entry((*part).to_string()).or_insert(Value::Null);
    }
}

/// `current` with every field of `patch` (a partial `settings.json`, any depth) replaced, then
/// checked: a value of the wrong type, `remember_answers_days` above 7, a copy timeout outside
/// 100 to 5,000 ms or a restore delay outside 50 to 5,000 ms is `Invalid {field}`. `version` is
/// not a setting and is ignored.
pub fn apply_patch(current: &Settings, patch: &Value) -> Result<Settings, CommandError> {
    if !patch.is_object() {
        return Err(invalid_field("settings"));
    }
    let mut all = Vec::new();
    leaves("", patch, &mut all);
    let base = serde_json::to_value(current).expect("settings serialise");
    let mut merged = base.clone();
    for (path, value) in &all {
        if path == "version" {
            continue;
        }
        let mut alone = base.clone();
        set_path(&mut alone, path, value.clone());
        if serde_json::from_value::<Settings>(alone).is_err() {
            return Err(invalid_field(path));
        }
        set_path(&mut merged, path, value.clone());
    }
    let next: Settings = serde_json::from_value(merged).map_err(|_| invalid_field("settings"))?;
    if next.privacy.remember_answers_days > 7 {
        return Err(invalid_field("privacy.remember_answers_days"));
    }
    if !(100..=5_000).contains(&next.capture.copy_timeout_ms) {
        return Err(invalid_field("capture.copy_timeout_ms"));
    }
    if !(50..=5_000).contains(&next.capture.paste_restore_delay_ms) {
        return Err(invalid_field("capture.paste_restore_delay_ms"));
    }
    Ok(next)
}

/// Whether `url` is an address the own key may use: `https://` anywhere, `http://` only on this
/// computer.
pub fn own_key_url_ok(url: &str) -> bool {
    let url = url.trim();
    if url_host(url).is_none() || url.chars().any(char::is_whitespace) {
        return false;
    }
    let scheme = url.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("https") => true,
        Some("http") => is_local_url(url),
        _ => false,
    }
}

/// What `settings_update`, `app_pause` and the own key commands do, without Tauri.
pub struct SettingsService {
    settings: SharedSettings,
    config_dir: PathBuf,
    hotkeys: Mutex<Hotkeys>,
    /// The system's refusal of the check shortcut at start, until a change or resume registers
    /// it.
    problem: Mutex<Option<HotkeyError>>,
    registrar: Arc<dyn HotkeyRegistrar>,
    layout: Arc<dyn KeyboardLayout>,
    ai: Option<Arc<AiClient>>,
}

impl std::fmt::Debug for SettingsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsService")
            .field("hotkeys", &*self.hotkeys())
            .finish_non_exhaustive()
    }
}

impl SettingsService {
    /// A service over these settings, saved under `config_dir`, with the check shortcut from
    /// the settings (not registered until [`SettingsService::start`]).
    pub fn new(
        settings: SharedSettings,
        config_dir: impl Into<PathBuf>,
        registrar: Arc<dyn HotkeyRegistrar>,
        layout: Arc<dyn KeyboardLayout>,
    ) -> Self {
        let check = settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .hotkeys
            .check
            .clone();
        Self {
            settings,
            config_dir: config_dir.into(),
            hotkeys: Mutex::new(Hotkeys::new(&check)),
            problem: Mutex::new(None),
            registrar,
            layout,
            ai: None,
        }
    }

    /// The same service, telling `ai` about privacy changes.
    pub fn with_ai(mut self, ai: Arc<AiClient>) -> Self {
        self.ai = Some(ai);
        self
    }

    fn hotkeys(&self) -> MutexGuard<'_, Hotkeys> {
        self.hotkeys.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn problem(&self) -> MutexGuard<'_, Option<HotkeyError>> {
        self.problem.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Why the check shortcut is not registered, when the system refused it at start.
    pub fn hotkey_problem(&self) -> Option<HotkeyError> {
        self.problem().clone()
    }

    /// The hotkeys as they are now.
    pub fn hotkey_state(&self) -> Hotkeys {
        self.hotkeys().clone()
    }

    /// The registrar in use.
    pub fn registrar(&self) -> &Arc<dyn HotkeyRegistrar> {
        &self.registrar
    }

    /// The config directory.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// The settings now.
    pub fn get(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `settings_get`.
    pub fn view(&self) -> SettingsView {
        let settings = self.get();
        let own_key_saved = Secrets::load(&self.config_dir)
            .ok()
            .and_then(|s| s.own_key)
            .is_some_and(|k| !redpen_core::provider::ExposeSecret::expose_secret(&k).is_empty());
        SettingsView {
            read_only: settings.read_only(),
            settings,
            own_key_saved,
            hotkey_problem: self.hotkey_problem().map(|e| e.to_string()),
        }
    }

    /// At start: registers the check shortcut unless Redpen is paused. A refusal is kept, so
    /// Settings → General can say why the shortcut does nothing.
    pub fn start(&self) -> Result<(), HotkeyError> {
        let paused = self.get().ui.paused;
        let result = self.hotkeys().start(self.registrar.as_ref(), paused);
        *self.problem() = result.as_ref().err().cloned();
        result
    }

    /// `settings_update`: applies `patch` (see [`apply_patch`]), changes the shortcut and the
    /// pause state when they differ, saves `settings.json`, and returns the settings. The
    /// settings stay locked from reading to saving, so a change made at the same time elsewhere
    /// (`set_providers`) is never lost.
    pub fn update(&self, patch: &Value) -> Result<Settings, CommandError> {
        let mut hotkeys = self.hotkeys();
        let mut shared = self.settings.write().unwrap_or_else(|e| e.into_inner());
        let current = shared.clone();
        if current.read_only() {
            return Err(CommandError::invalid(
                redpen_core::store::SettingsError::NewerVersion.to_string(),
            ));
        }
        let mut next = apply_patch(&current, patch)?;
        let before = hotkeys.clone();
        // A shortcut the system refused at start is asked for again when it is set again.
        let retry = self.problem().is_some() && patch.pointer("/hotkeys/check").is_some();
        if next.hotkeys.check != current.hotkeys.check || retry {
            hotkeys.change(
                &next.hotkeys.check,
                self.registrar.as_ref(),
                self.layout.as_ref(),
            )?;
            next.hotkeys.check = hotkeys.check().to_string();
        }
        if next.ui.paused != hotkeys.is_paused() {
            if next.ui.paused {
                hotkeys.pause(self.registrar.as_ref());
            } else if let Err(e) = hotkeys.resume(self.registrar.as_ref()) {
                // Stay paused: nothing is registered, and the setting says so.
                hotkeys.pause(self.registrar.as_ref());
                return Err(e.into());
            }
        }
        if let Err(e) = next.save(&self.config_dir) {
            self.undo_hotkeys(&mut hotkeys, &before);
            return Err(CommandError::invalid(e.to_string()));
        }
        *shared = next.clone();
        // Registered now (changed, asked for again, or resumed), or let go of on purpose.
        let settled = hotkeys.check() != before.check() || retry;
        if settled || hotkeys.is_paused() || before.is_paused() {
            *self.problem() = None;
        }
        if let Some(ai) = &self.ai {
            ai.set_privacy(PrivacyOptions::from(&next.privacy));
        }
        tracing::info!("settings_updated");
        Ok(next)
    }

    /// Puts the registrations back as they were in `before`.
    fn undo_hotkeys(&self, hotkeys: &mut Hotkeys, before: &Hotkeys) {
        let registrar = self.registrar.as_ref();
        if hotkeys.check() != before.check() && !hotkeys.is_paused() {
            registrar.unregister(hotkeys.check());
        }
        *hotkeys = before.clone();
        if before.is_paused() {
            hotkeys.pause(registrar);
        } else if hotkeys.resume(registrar).is_err() {
            tracing::warn!("a hotkey could not be registered again");
        }
    }

    /// `app_pause`: Pause Redpen (every hotkey unregistered) or resume, saved as `ui.paused`.
    pub fn set_paused(&self, paused: bool) -> Result<Settings, CommandError> {
        self.update(&json!({"ui": {"paused": paused}}))
    }

    /// Marks the welcome window as shown, so it never opens again.
    pub fn mark_welcomed(&self) -> Result<Settings, CommandError> {
        self.update(&json!({"ui": {"welcomed": true}}))
    }

    /// `own_key_save`: the address and model into `settings.json`, the key into
    /// `secrets.json`. `InvalidUrl` for an address that is not `https://` (or `http://` on this
    /// computer); `Invalid {field}` for an empty key or model.
    pub fn own_key_save(&self, input: &OwnKeyInput) -> Result<(), CommandError> {
        let base_url = input.base_url.trim();
        if !own_key_url_ok(base_url) {
            return Err(CommandError(
                json!({"kind": "InvalidUrl", "message": INVALID_URL}),
            ));
        }
        let key = input.key.trim();
        let model = input.chat_model.trim();
        if model.is_empty() {
            return Err(invalid_field("providers.own_key.chat_model"));
        }
        let mut secrets = Secrets::load(&self.config_dir).unwrap_or_default();
        if !key.is_empty() {
            secrets.own_key = Some(SecretString::from(key.to_string()));
        } else if secrets.own_key.is_none() {
            return Err(invalid_field("own_key"));
        }
        self.update(
            &json!({"providers": {"own_key": {"base_url": base_url, "chat_model": model}}}),
        )?;
        secrets
            .save(&self.config_dir)
            .map_err(|_| CommandError::invalid(KEY_NOT_SAVED))?;
        tracing::info!("own_key_saved");
        Ok(())
    }

    /// `own_key_clear`: forgets the key; the address and model stay.
    pub fn own_key_clear(&self) -> Result<(), CommandError> {
        let mut secrets = Secrets::load(&self.config_dir).unwrap_or_default();
        secrets.own_key = None;
        secrets
            .save(&self.config_dir)
            .map_err(|_| CommandError::invalid(KEY_NOT_REMOVED))?;
        tracing::info!("own_key_cleared");
        Ok(())
    }

    /// The provider `provider_test` tests: the saved own key, or the local server.
    pub fn test_target(&self, kind: &str) -> Result<ResolvedProvider, ProviderTest> {
        let settings = self.get();
        match kind {
            "own_key" => {
                let secrets = Secrets::load(&self.config_dir).unwrap_or_default();
                ResolvedProvider::from_own_key_settings(&settings.providers.own_key, &secrets)
                    .map_err(|e| ProviderTest {
                        ok: false,
                        message: e.to_string(),
                    })
            }
            "local" => Ok(ResolvedProvider::from_local_settings(
                &settings.providers.local,
            )),
            _ => Err(ProviderTest {
                ok: false,
                message: NOTHING_TO_TEST.into(),
            }),
        }
    }
}

/// `provider_test`: the Test button's answer for `kind`. A kind with nothing to test, or an own
/// key that is not set up, is answered here and sends nothing.
pub async fn test_provider(settings: &SettingsService, ai: &AiClient, kind: &str) -> ProviderTest {
    match settings.test_target(kind) {
        Ok(p) => ai.provider_test(&p).await,
        Err(result) => result,
    }
}

/// `sent_log_clear`: empties the sent log.
pub fn clear_sent_log(ai: &AiClient) -> Result<(), CommandError> {
    ai.sent_log_clear()
        .map_err(|_| CommandError::invalid(LOG_NOT_CLEARED))
}

/// `cache_clear`: forgets every remembered answer.
pub fn clear_cache(ai: &AiClient) -> Result<(), CommandError> {
    ai.cache_clear()
        .map_err(|_| CommandError::invalid(CACHE_NOT_CLEARED))
}

/// `open_settings`: the target for `tab` and `section` as the interface names them (General for
/// an unknown tab, no section outside Providers), opened by `open`.
pub fn open_settings_at<E>(
    tab: &str,
    section: Option<&str>,
    open: impl FnOnce(&SettingsTarget) -> Result<(), E>,
) -> Result<(), CommandError> {
    open(&SettingsTarget::new(tab, section)).map_err(|_| CommandError::invalid(SETTINGS_NOT_OPENED))
}

/// This month's publik API use (the local month of `now_ms`), from `usage_daily`; the Test
/// buttons' calls are not counted.
pub fn month_usage(ai: &AiClient, clock: &dyn Clock) -> UsageMonth {
    let day = local_day(clock.now_ms(), clock.utc_offset_minutes());
    let month = &day[..7];
    let mut out = UsageMonth::default();
    let rows = ai.db().usage_rows().unwrap_or_default();
    for row in rows
        .iter()
        .filter(|r| r.day.starts_with(month) && r.provider == "publik" && r.feature != "test")
    {
        out.total_micros = out.total_micros.saturating_add(row.charge_micros);
        out.calls = out.calls.saturating_add(row.calls);
        match out.by_feature.iter_mut().find(|f| f.feature == row.feature) {
            Some(f) => {
                f.calls += row.calls;
                f.charge_micros += row.charge_micros;
            }
            None => out.by_feature.push(FeatureUsage {
                feature: row.feature.clone(),
                calls: row.calls,
                charge_micros: row.charge_micros,
            }),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The commands.

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> SettingsView {
    state.settings.view()
}

/// A change in Settings: the merged settings, or the field (or shortcut) that was refused. The
/// tray follows the pause state, and Harper is made ready for a new dialect or categories.
#[tauri::command]
pub fn settings_update(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: Value,
) -> Result<Settings, CommandError> {
    let next = state.settings.update(&patch)?;
    crate::show_paused(&app, next.ui.paused);
    crate::warm_checker(&state, next.clone());
    Ok(next)
}

#[tauri::command]
pub fn own_key_save(state: State<'_, AppState>, input: OwnKeyInput) -> Result<(), CommandError> {
    state.settings.own_key_save(&input)
}

#[tauri::command]
pub fn own_key_clear(state: State<'_, AppState>) -> Result<(), CommandError> {
    state.settings.own_key_clear()
}

/// The Test buttons: `GET {base_url}/models` with the own key, or Ollama's model list.
#[tauri::command]
pub async fn provider_test(
    state: State<'_, AppState>,
    kind: String,
) -> Result<ProviderTest, CommandError> {
    Ok(test_provider(&state.settings, &state.ai, &kind).await)
}

#[tauri::command]
pub async fn local_models(state: State<'_, AppState>) -> Result<Vec<LocalModel>, CommandError> {
    let base_url = state.settings.get().providers.local.chat_base_url;
    Ok(state.ai.local_models(&base_url).await?)
}

#[tauri::command]
pub fn usage_month(state: State<'_, AppState>) -> UsageMonth {
    month_usage(&state.ai, state.clock.as_ref())
}

#[tauri::command]
pub fn sent_log_list(state: State<'_, AppState>) -> Vec<SentEntry> {
    state.ai.db().sent_log_list().unwrap_or_default()
}

#[tauri::command]
pub fn sent_log_clear(state: State<'_, AppState>) -> Result<(), CommandError> {
    clear_sent_log(&state.ai)
}

#[tauri::command]
pub fn cache_clear(state: State<'_, AppState>) -> Result<(), CommandError> {
    clear_cache(&state.ai)
}

/// Pause Redpen, from Settings or the tray: every hotkey let go of (or registered again), the
/// tray icon greyed (or not), saved as `ui.paused`.
#[tauri::command]
pub fn app_pause(app: AppHandle, paused: bool) -> Result<(), CommandError> {
    crate::set_paused(&app, paused).map(|_| ())
}

/// Opens Settings at a tab (and a section of Providers): from the popup's consent view and
/// first-run card, and from the welcome window. Unknown names open General.
#[tauri::command]
pub fn open_settings(
    app: AppHandle,
    tab: String,
    section: Option<String>,
) -> Result<(), CommandError> {
    open_settings_at(&tab, section.as_deref(), |target| {
        crate::windows::open_settings(&app, target)
    })
}

/// The welcome window's Close.
#[tauri::command]
pub fn welcome_done(app: AppHandle) {
    crate::windows::close_window(&app, crate::windows::WELCOME);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::NoAltGr;
    use std::sync::RwLock;

    #[derive(Default)]
    struct Fake {
        refuse: Vec<String>,
        registered: Mutex<Vec<String>>,
    }

    impl HotkeyRegistrar for Fake {
        fn register(&self, hotkey: &str) -> Result<(), String> {
            if self.refuse.iter().any(|r| r == hotkey) {
                return Err("in use".into());
            }
            self.registered.lock().unwrap().push(hotkey.into());
            Ok(())
        }
        fn unregister(&self, hotkey: &str) {
            self.registered.lock().unwrap().retain(|h| h != hotkey);
        }
        fn registered(&self) -> Vec<String> {
            self.registered.lock().unwrap().clone()
        }
    }

    fn service(dir: &Path, fake: Arc<Fake>) -> SettingsService {
        let settings: SharedSettings = Arc::new(RwLock::new(Settings::default()));
        let s = SettingsService::new(settings, dir, fake, Arc::new(NoAltGr));
        s.start().unwrap();
        s
    }

    #[test]
    fn a_patch_changes_only_its_fields_and_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let s = service(dir.path(), Arc::new(Fake::default()));
        let next = s
            .update(&json!({"check": {"dialect": "british"}, "privacy": {"sent_log": true}}))
            .unwrap();
        assert_eq!(next.check.dialect, redpen_core::check::Dialect::British);
        assert!(next.privacy.sent_log);
        assert!(next.privacy.redaction);
        assert_eq!(Settings::load(dir.path()).unwrap(), next);
    }

    #[test]
    fn bad_values_name_their_field() {
        let dir = tempfile::tempdir().unwrap();
        let s = service(dir.path(), Arc::new(Fake::default()));
        for (patch, field) in [
            (
                json!({"privacy": {"remember_answers_days": 30}}),
                "privacy.remember_answers_days",
            ),
            (
                json!({"privacy": {"redaction": "yes"}}),
                "privacy.redaction",
            ),
            (json!({"check": {"dialect": "klingon"}}), "check.dialect"),
            (
                json!({"capture": {"copy_timeout_ms": 10}}),
                "capture.copy_timeout_ms",
            ),
        ] {
            let e = s.update(&patch).unwrap_err();
            assert_eq!(e.kind(), Some("Invalid"));
            assert_eq!(e.0["field"], field);
        }
        assert!(!dir.path().join("settings.json").exists());
    }

    #[test]
    fn a_refused_hotkey_keeps_the_old_one_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake {
            refuse: vec!["Ctrl+Alt+K".into()],
            ..Fake::default()
        });
        let s = service(dir.path(), fake.clone());
        let e = s
            .update(&json!({"hotkeys": {"check": "Ctrl+Alt+K"}}))
            .unwrap_err();
        assert_eq!(
            e.0,
            json!({"kind": "HotkeyInUse",
                   "message": "Ctrl+Alt+K is already used by another app. Choose another shortcut.",
                   "hotkey": "Ctrl+Alt+K"})
        );
        assert_eq!(fake.registered(), ["Ctrl+Alt+R"]);
        assert_eq!(s.get().hotkeys.check, "Ctrl+Alt+R");
        let next = s
            .update(&json!({"hotkeys": {"check": "ctrl+alt+j"}}))
            .unwrap();
        assert_eq!(next.hotkeys.check, "Ctrl+Alt+J");
        assert_eq!(fake.registered(), ["Ctrl+Alt+J"]);
    }

    /// Refuses `Ctrl+Alt+R` while another app holds it.
    #[derive(Default)]
    struct Held {
        held_elsewhere: std::sync::atomic::AtomicBool,
        registered: Mutex<Vec<String>>,
    }

    impl HotkeyRegistrar for Held {
        fn register(&self, hotkey: &str) -> Result<(), String> {
            if self
                .held_elsewhere
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err("in use".into());
            }
            self.registered.lock().unwrap().push(hotkey.into());
            Ok(())
        }
        fn unregister(&self, hotkey: &str) {
            self.registered.lock().unwrap().retain(|h| h != hotkey);
        }
        fn registered(&self) -> Vec<String> {
            self.registered.lock().unwrap().clone()
        }
    }

    #[test]
    fn a_shortcut_refused_at_start_is_reported_until_it_is_registered() {
        let dir = tempfile::tempdir().unwrap();
        let held = Arc::new(Held::default());
        held.held_elsewhere
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let settings: SharedSettings = Arc::new(RwLock::new(Settings::default()));
        let s = SettingsService::new(settings, dir.path(), held.clone(), Arc::new(NoAltGr));
        assert!(s.start().is_err());
        assert_eq!(
            s.view().hotkey_problem.as_deref(),
            Some("Ctrl+Alt+R is already used by another app. Choose another shortcut.")
        );
        // An unrelated change neither asks again nor clears it.
        s.update(&json!({"check": {"dialect": "american"}}))
            .unwrap();
        assert!(s.view().hotkey_problem.is_some());
        // The other app let go: setting the same shortcut again registers it.
        held.held_elsewhere
            .store(false, std::sync::atomic::Ordering::SeqCst);
        s.update(&json!({"hotkeys": {"check": "Ctrl+Alt+R"}}))
            .unwrap();
        assert_eq!(held.registered(), ["Ctrl+Alt+R"]);
        assert_eq!(s.view().hotkey_problem, None);
    }

    #[test]
    fn pause_unregisters_and_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake::default());
        let s = service(dir.path(), fake.clone());
        assert!(s.set_paused(true).unwrap().ui.paused);
        assert!(fake.registered().is_empty());
        assert!(Settings::load(dir.path()).unwrap().ui.paused);
        s.set_paused(false).unwrap();
        assert_eq!(fake.registered(), ["Ctrl+Alt+R"]);
    }

    #[test]
    fn own_key_addresses_must_be_https_or_this_computer() {
        assert!(own_key_url_ok("https://api.example.com/v1"));
        assert!(own_key_url_ok("http://localhost:8080/v1"));
        assert!(own_key_url_ok("http://127.0.0.1:1234/v1"));
        assert!(!own_key_url_ok("http://api.example.com/v1"));
        assert!(!own_key_url_ok("ftp://api.example.com"));
        assert!(!own_key_url_ok("api.example.com"));
        let dir = tempfile::tempdir().unwrap();
        let s = service(dir.path(), Arc::new(Fake::default()));
        let e = s
            .own_key_save(&OwnKeyInput {
                base_url: "http://api.example.com/v1".into(),
                key: "sk-test-a".into(),
                chat_model: "example-chat".into(),
            })
            .unwrap_err();
        assert_eq!(
            e.0,
            json!({"kind": "InvalidUrl",
                   "message": "Use an https:// address (or http://localhost for a server on this computer)."})
        );
    }

    #[test]
    fn a_saved_own_key_is_never_sent_back() {
        let dir = tempfile::tempdir().unwrap();
        let s = service(dir.path(), Arc::new(Fake::default()));
        assert!(!s.view().own_key_saved);
        s.own_key_save(&OwnKeyInput {
            base_url: "https://api.example.com/v1".into(),
            key: "sk-test-a".into(),
            chat_model: "example-chat".into(),
        })
        .unwrap();
        let view = s.view();
        assert!(view.own_key_saved);
        assert_eq!(view.settings.providers.own_key.chat_model, "example-chat");
        let text = serde_json::to_string(&view).unwrap();
        assert!(!text.contains("sk-test-a"), "{text}");
        assert!(text.contains("\"own_key_saved\":true"));
        s.own_key_clear().unwrap();
        assert!(!s.view().own_key_saved);
        let debug = format!(
            "{:?}",
            OwnKeyInput {
                base_url: String::new(),
                key: "sk-test-a".into(),
                chat_model: String::new()
            }
        );
        assert!(!debug.contains("sk-test-a"));
    }
}
