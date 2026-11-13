//! Module 06 tests in the app crate (docs/modules/06-popup-and-settings-ui.md, Acceptance tests):
//! the check shortcut and its refusals, Pause, the Welcome flag, the own key, the settings
//! commands' refusals, this month's usage, the tray menu and the popup latency check. A test named
//! `tNN_…` covers bullet NN (12, 13, 15 and 17), `e_…` an error path the Scope introduces, and
//! `s_…` a Scope bullet the numbered list leaves implicit.
//!
//! Expected values come from the spec (the Messages, the Design notes' `HotkeyRegistrar`,
//! `KeyboardLayout` and `Hotkeys::change`, the shortcut grammar, `settings_update`, the own key,
//! the settings commands' refusals, `usage_month` and Linux), docs/ARCHITECTURE.md §4, §5, §8 and
//! §10 (settings, the commands' errors, the requests the app can make, AltGr), docs/PRODUCT.md §5
//! (Welcome, the tray menu), the Polish programmer's keyboard layout, on which Ctrl+Alt+letter
//! types the Polish letters, and calendar arithmetic for the usage month
//! (tests/cases/06-popup-and-settings-ui/REFERENCE.md). Nothing here is taken from Redpen's own
//! output.
//!
//! The registrar and the layout are fakes written here against the two traits the Design notes
//! give. The spec names `Hotkeys::change`, `pause` and `resume` but not how the commands around
//! them are built; those calls are gathered in `mod api`, so a different name changes one line
//! there and no expected value.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use redpen_app::hotkeys::{HotkeyRegistrar, KeyboardLayout};
use redpen_core::clock::{FakeClock, FakeSleeper};
use redpen_core::flow::Sessions;
use redpen_core::net::FakeTransport;
use redpen_core::platform::FakeCapture;
use redpen_core::provider::{AiClient, AiError, FixedResolver, NoopStatusSink};
use redpen_core::store::{CacheRow, Db, Secrets, Settings};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// The calls the spec describes without naming them.

mod api {
    use std::path::Path;
    use std::sync::{Arc, RwLock};

    use redpen_app::commands::session::open_on_shortcut;
    use redpen_app::commands::settings::{
        OwnKeyInput, SettingsService, clear_cache, clear_sent_log, month_usage, open_settings_at,
        test_provider,
    };
    use redpen_app::hotkeys::{HotkeyRegistrar, Hotkeys, KeyboardLayout};
    use redpen_app::publik::{PublikOptions, PublikService, StatusStore};
    use redpen_core::clock::{Clock, FakeClock};
    use redpen_core::flow::Sessions;
    use redpen_core::net::HttpTransport;
    use redpen_core::provider::AiClient;
    use redpen_core::store::Settings;
    use serde::Serialize;
    use serde_json::Value;

    fn json<T: Serialize>(value: &T) -> Value {
        serde_json::to_value(value).expect("the value serialises")
    }

    /// The check shortcut `check`, registered with `registrar` as the app does at start.
    pub fn started(check: &str, registrar: &dyn HotkeyRegistrar) -> Result<Hotkeys, Value> {
        let mut hotkeys = Hotkeys::new(check);
        hotkeys
            .start(registrar, false)
            .map(|()| hotkeys)
            .map_err(|e| json(&e))
    }

    /// `Hotkeys::change`; the refusal as the interface receives it, `{kind, message, ...}`.
    pub fn change(
        hotkeys: &mut Hotkeys,
        new: &str,
        registrar: &dyn HotkeyRegistrar,
        layout: &dyn KeyboardLayout,
    ) -> Result<(), Value> {
        hotkeys.change(new, registrar, layout).map_err(|e| json(&e))
    }

    pub fn pause(hotkeys: &mut Hotkeys, registrar: &dyn HotkeyRegistrar) {
        hotkeys.pause(registrar);
    }

    pub fn resume(hotkeys: &mut Hotkeys, registrar: &dyn HotkeyRegistrar) -> Result<(), Value> {
        hotkeys.resume(registrar).map_err(|e| json(&e))
    }

    /// What the settings commands work on, as the app builds it at start: settings.json from
    /// `config_dir`, the check shortcut registered (unless Redpen is paused).
    pub fn service(
        config_dir: &Path,
        registrar: Arc<dyn HotkeyRegistrar>,
        layout: Arc<dyn KeyboardLayout>,
    ) -> Result<SettingsService, Value> {
        let settings = Settings::load(config_dir).expect("settings.json loads");
        let service = SettingsService::new(
            Arc::new(RwLock::new(settings)),
            config_dir,
            registrar,
            layout,
        );
        service.start().map(|()| service).map_err(|e| json(&e))
    }

    /// `settings_get`, as the interface receives it.
    pub fn settings_get(s: &SettingsService) -> Value {
        json(&s.view())
    }

    /// `settings_update(patch)`.
    pub fn settings_update(s: &SettingsService, patch: Value) -> Result<Value, Value> {
        s.update(&patch).map(|x| json(&x)).map_err(|e| json(&e))
    }

    /// `app_pause(paused)`: the tray's Pause Redpen and its undoing.
    pub fn app_pause(s: &SettingsService, paused: bool) -> Result<(), Value> {
        s.set_paused(paused).map(|_| ()).map_err(|e| json(&e))
    }

    /// Whether the app opens the Welcome window at start with these settings.
    pub fn shows_welcome(settings: &Settings) -> bool {
        redpen_app::windows::shows_welcome(settings)
    }

    /// The Welcome window's Close: remembered in settings.json.
    pub fn welcome_closed(s: &SettingsService) {
        s.mark_welcomed().expect("the welcomed flag is saved");
    }

    /// `own_key_save({base_url, key, chat_model})`.
    pub fn own_key_save(
        s: &SettingsService,
        base_url: &str,
        key: &str,
        chat_model: &str,
    ) -> Result<(), Value> {
        let input = OwnKeyInput {
            base_url: base_url.to_string(),
            key: key.to_string(),
            chat_model: chat_model.to_string(),
        };
        s.own_key_save(&input).map_err(|e| json(&e))
    }

    /// `own_key_clear`: forgets the saved key.
    pub fn own_key_clear(s: &SettingsService) -> Result<(), Value> {
        s.own_key_clear().map_err(|e| json(&e))
    }

    /// `provider_test(kind)`: the Test button's `{ok, message}`.
    pub async fn provider_test(s: &SettingsService, ai: &AiClient, kind: &str) -> Value {
        json(&test_provider(s, ai, kind).await)
    }

    /// `usage_month`: this month's publik API use at the clock's time.
    pub fn usage_month(ai: &AiClient, clock: &dyn Clock) -> Value {
        json(&month_usage(ai, clock))
    }

    /// `sent_log_clear`.
    pub fn sent_log_clear(ai: &AiClient) -> Result<(), Value> {
        clear_sent_log(ai).map_err(|e| json(&e))
    }

    /// `cache_clear` (Settings → Privacy → Clear now).
    pub fn cache_clear(ai: &AiClient) -> Result<(), Value> {
        clear_cache(ai).map_err(|e| json(&e))
    }

    /// `open_settings(tab, section?)`, with `open` standing for the window: it receives the
    /// target as JSON and says whether the window could be made or shown.
    pub fn open_settings(
        tab: &str,
        section: Option<&str>,
        open: impl FnOnce(Value) -> Result<(), ()>,
    ) -> Result<(), Value> {
        open_settings_at(tab, section, |target| open(json(target))).map_err(|e| json(&e))
    }

    /// A press of the check shortcut, on a system that can (`can_capture`) or cannot read a
    /// selection: the session as `session_get` returns it, or what `popup://open` carries.
    pub async fn shortcut(
        sessions: &Sessions,
        settings: &Settings,
        can_capture: bool,
    ) -> Result<Value, Value> {
        open_on_shortcut(sessions, settings, can_capture)
            .await
            .map(|s| json(&s))
    }

    /// The tray menu's items, label and whether each is enabled, top to bottom.
    pub fn tray_menu(balance: Option<&str>) -> Vec<(String, bool)> {
        redpen_app::tray::menu_items(balance)
            .into_iter()
            .map(|item| (item.label, item.enabled))
            .collect()
    }

    /// What the app does with publik API at start (module 03): `GET /wallet` only while publik
    /// API is chosen and set up. A fresh install: no credential (its path is in `data_dir`, where
    /// none was written), no app token and no `PUBLIK_API_*` variable.
    pub async fn publik_at_start(t: Arc<dyn HttpTransport>, config_dir: &Path, data_dir: &Path) {
        let settings = Settings::load(config_dir).expect("settings.json loads");
        let options = PublikOptions {
            credential_path: data_dir.join("publik-credential.json"),
            app_token: None,
            env: |_| None,
            ..PublikOptions::current()
        };
        let service = PublikService::new(
            t,
            options,
            config_dir.to_path_buf(),
            Arc::new(RwLock::new(settings)),
            Arc::new(StatusStore::load(
                data_dir,
                Arc::new(FakeClock::at(1_790_000_000_000)),
            )),
        );
        let _ = service.refresh_at_start().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Expected values.

/// The default check shortcut (docs/ARCHITECTURE.md §4).
const DEFAULT: &str = "Ctrl+Alt+R";
/// A shortcut the fake registrar is told another app holds.
const TAKEN: &str = "Ctrl+Alt+K";
/// A free shortcut on every layout here.
const FREE: &str = "Ctrl+Alt+J";
/// Ctrl+Alt+C types `ć` (U+0107) on the Polish programmer's layout (docs/ARCHITECTURE.md §10).
const ALTGR_C: &str = "Ctrl+Alt+C";
/// Ctrl+Alt+E types `ę` (U+0119) on the same layout.
const ALTGR_E: &str = "Ctrl+Alt+E";

/// The spec's Messages: `HotkeyInUse` names the shortcut it refused.
fn in_use(hotkey: &str) -> String {
    format!("{hotkey} is already used by another app. Choose another shortcut.")
}
const MSG_ALTGR: &str = "That shortcut types a character on your keyboard layout. Choose another.";
const MSG_INVALID_URL: &str =
    "Use an https:// address (or http://localhost for a server on this computer).";
/// The Design notes' shortcut grammar.
const MSG_HOTKEY_INVALID: &str =
    "Use Ctrl, Alt or the Windows key, with Shift if you like, and one letter, digit or F1 to F24.";
/// The Design notes' `settings_update`: a file written by a newer Redpen.
const MSG_NEWER: &str = "Settings were written by a newer Redpen.";
/// The Design notes' Linux sentence.
const MSG_CAPTURE_UNAVAILABLE: &str =
    "Redpen can't read a selection on this system yet. Checking selections works on Windows.";

/// The Design notes' `Invalid {field}`: the message names the field by its dotted path.
fn invalid(field: &str) -> Value {
    json!({"kind": "Invalid", "message": format!("That value can't be used for {field}."),
           "field": field})
}

/// The Design notes' refusals of the settings commands: `Invalid` with the message alone.
fn refused(message: &str) -> Value {
    json!({"kind": "Invalid", "message": message})
}

/// The own key of these tests: short, so it matches no secret shape of the hygiene test.
const OWN_KEY: &str = "sk-test-six";
const OWN_MODEL: &str = "example-chat";

// ---------------------------------------------------------------------------------------------
// Fakes.

/// A shortcut as the operating system sees it: case and spaces do not matter.
fn norm(hotkey: &str) -> String {
    hotkey
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn set(hotkeys: &[&str]) -> BTreeSet<String> {
    hotkeys.iter().map(|h| norm(h)).collect()
}

/// The global shortcut service: holds what it was asked to register, and refuses what another
/// app holds.
#[derive(Default)]
struct FakeRegistrar {
    held: Mutex<Vec<String>>,
    others: Mutex<Vec<String>>,
}

impl FakeRegistrar {
    /// A registrar on which another app already holds `hotkeys`.
    fn with_others(hotkeys: &[&str]) -> Self {
        let r = Self::default();
        *r.others.lock().unwrap() = hotkeys.iter().map(|h| norm(h)).collect();
        r
    }

    /// What it holds now, compared as the operating system compares shortcuts.
    fn holding(&self) -> BTreeSet<String> {
        self.registered().iter().map(|h| norm(h)).collect()
    }
}

impl HotkeyRegistrar for FakeRegistrar {
    fn register(&self, hotkey: &str) -> Result<(), String> {
        if self.others.lock().unwrap().contains(&norm(hotkey)) {
            return Err(format!("{hotkey} is registered by another application"));
        }
        let mut held = self.held.lock().unwrap();
        if !held.iter().any(|h| norm(h) == norm(hotkey)) {
            held.push(hotkey.to_string());
        }
        Ok(())
    }

    fn unregister(&self, hotkey: &str) {
        self.held
            .lock()
            .unwrap()
            .retain(|h| norm(h) != norm(hotkey));
    }

    fn registered(&self) -> Vec<String> {
        self.held.lock().unwrap().clone()
    }
}

/// A keyboard layout: what Ctrl+Alt+letter types on it, if anything.
struct FakeLayout {
    altgr: &'static [(char, char)],
}

/// The US layout: no Ctrl+Alt combination types a character.
const US: FakeLayout = FakeLayout { altgr: &[] };

/// The Polish programmer's layout: Ctrl+Alt (AltGr) with these letters types the Polish ones.
const POLISH: FakeLayout = FakeLayout {
    altgr: &[
        ('A', '\u{0105}'), // ą
        ('C', '\u{0107}'), // ć
        ('E', '\u{0119}'), // ę
        ('L', '\u{0142}'), // ł
        ('N', '\u{0144}'), // ń
        ('O', '\u{00F3}'), // ó
        ('S', '\u{015B}'), // ś
        ('X', '\u{017A}'), // ź
        ('Z', '\u{017C}'), // ż
    ],
};

impl KeyboardLayout for FakeLayout {
    fn types_character(&self, hotkey: &str) -> Option<char> {
        let parts: Vec<String> = hotkey
            .split('+')
            .map(|p| p.trim().to_ascii_lowercase())
            .collect();
        let (key, modifiers) = parts.split_last()?;
        let modifiers: BTreeSet<&str> = modifiers.iter().map(String::as_str).collect();
        let ctrl_alt: BTreeSet<&str> = ["ctrl", "alt"].into_iter().collect();
        if modifiers != ctrl_alt || key.chars().count() != 1 {
            return None;
        }
        let letter = key.chars().next()?.to_ascii_uppercase();
        self.altgr
            .iter()
            .find(|(k, _)| *k == letter)
            .map(|(_, typed)| *typed)
    }
}

fn kind(e: &Value) -> &str {
    e["kind"].as_str().unwrap_or_default()
}

fn message(e: &Value) -> &str {
    e["message"].as_str().unwrap_or_default()
}

fn on_disk(config_dir: &Path) -> Settings {
    Settings::load(config_dir).expect("settings.json loads")
}

fn temp() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary folder")
}

fn service(
    config_dir: &Path,
    registrar: &Arc<FakeRegistrar>,
    layout: FakeLayout,
) -> redpen_app::commands::settings::SettingsService {
    api::service(config_dir, registrar.clone(), Arc::new(layout))
        .unwrap_or_else(|e| panic!("the app could not start: {e}"))
}

// ---------------------------------------------------------------------------------------------
// Bullet 12: a refused shortcut, through `Hotkeys::change` and through `settings_update`.

// Bullet 12: a shortcut the fake registrar refuses → `HotkeyInUse` naming it; the old shortcut
// stays registered.
#[test]
fn t12_a_shortcut_another_app_holds_is_refused_naming_it() {
    let registrar = FakeRegistrar::with_others(&[TAKEN]);
    let mut hotkeys = api::started(DEFAULT, &registrar).expect("Ctrl+Alt+R is free");
    assert_eq!(registrar.holding(), set(&[DEFAULT]));

    let err = api::change(&mut hotkeys, TAKEN, &registrar, &US).expect_err("another app holds it");
    assert_eq!(kind(&err), "HotkeyInUse");
    assert_eq!(message(&err), in_use(TAKEN));
    assert_eq!(
        norm(err["hotkey"].as_str().unwrap_or_default()),
        norm(TAKEN)
    );
    assert_eq!(
        registrar.holding(),
        set(&[DEFAULT]),
        "the old shortcut stays"
    );
}

// Bullet 12: a combination the fake layout maps to `ć` → the AltGr message; the old shortcut stays
// registered and the refused one is not.
#[test]
fn t12_a_combination_that_types_a_character_is_refused() {
    let registrar = FakeRegistrar::default();
    let mut hotkeys = api::started(DEFAULT, &registrar).expect("Ctrl+Alt+R is free");
    assert_eq!(POLISH.types_character(ALTGR_C), Some('\u{0107}'));

    for combination in [ALTGR_C, ALTGR_E] {
        let err = api::change(&mut hotkeys, combination, &registrar, &POLISH)
            .expect_err("it types a Polish letter");
        assert_eq!(message(&err), MSG_ALTGR, "{combination}");
        assert_eq!(registrar.holding(), set(&[DEFAULT]), "{combination}");
    }
}

// Bullet 12, the other way round: on a layout where Ctrl+Alt+C types nothing it is accepted, and
// the old shortcut is released.
#[test]
fn t12_the_same_combination_is_accepted_on_a_layout_without_altgr() {
    let registrar = FakeRegistrar::default();
    let mut hotkeys = api::started(DEFAULT, &registrar).expect("Ctrl+Alt+R is free");
    assert_eq!(US.types_character(ALTGR_C), None);

    api::change(&mut hotkeys, ALTGR_C, &registrar, &US).expect("Ctrl+Alt+C types nothing here");
    assert_eq!(registrar.holding(), set(&[ALTGR_C]));
}

// Bullet 12 through `settings_update`: the refusal reaches the interface as `HotkeyInUse
// {hotkey}`, and neither the registration nor settings.json changes.
#[test]
fn t12_settings_update_refuses_a_shortcut_in_use() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::with_others(&[TAKEN]));
    let s = service(dir.path(), &registrar, US);
    assert_eq!(registrar.holding(), set(&[DEFAULT]));

    let err = api::settings_update(&s, json!({"hotkeys": {"check": TAKEN}}))
        .expect_err("another app holds it");
    assert_eq!(kind(&err), "HotkeyInUse");
    assert_eq!(message(&err), in_use(TAKEN));
    assert_eq!(
        norm(err["hotkey"].as_str().unwrap_or_default()),
        norm(TAKEN)
    );

    assert_eq!(registrar.holding(), set(&[DEFAULT]));
    assert_eq!(api::settings_get(&s)["hotkeys"]["check"], DEFAULT);
    assert_eq!(on_disk(dir.path()).hotkeys.check, DEFAULT);
}

// Bullet 12 through `settings_update`, AltGr: the message, and nothing changes.
#[test]
fn t12_settings_update_refuses_an_altgr_combination() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, POLISH);

    let err = api::settings_update(&s, json!({"hotkeys": {"check": ALTGR_C}}))
        .expect_err("Ctrl+Alt+C types ć on this layout");
    assert_eq!(message(&err), MSG_ALTGR);
    assert_eq!(registrar.holding(), set(&[DEFAULT]));
    assert_eq!(api::settings_get(&s)["hotkeys"]["check"], DEFAULT);
    assert_eq!(on_disk(dir.path()).hotkeys.check, DEFAULT);
}

// Scope (changing the shortcut re-registers): a free shortcut replaces the old one, in the
// registrar and in settings.json.
#[test]
fn s_a_free_shortcut_replaces_the_old_one() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, POLISH);

    let saved = api::settings_update(&s, json!({"hotkeys": {"check": FREE}}))
        .unwrap_or_else(|e| panic!("Ctrl+Alt+J is free and types nothing: {e}"));
    assert_eq!(
        norm(saved["hotkeys"]["check"].as_str().unwrap_or_default()),
        norm(FREE)
    );
    assert_eq!(registrar.holding(), set(&[FREE]));
    assert_eq!(norm(&on_disk(dir.path()).hotkeys.check), norm(FREE));
}

// Error path (docs/ARCHITECTURE.md §5, `HotkeyInvalid`; the Design notes' shortcut grammar): a
// combination with no key, with no Ctrl, Alt or Win, or with a key outside the grammar is refused
// with the grammar's message, and nothing changes.
#[test]
fn e_a_shortcut_with_no_key_is_invalid() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);

    for refused in ["Ctrl+Alt+", "Shift+R", "Ctrl+Alt+F25", "Ctrl+Alt+Tab"] {
        let err =
            api::settings_update(&s, json!({"hotkeys": {"check": refused}})).expect_err(refused);
        assert_eq!(kind(&err), "HotkeyInvalid", "{refused}");
        assert_eq!(message(&err), MSG_HOTKEY_INVALID, "{refused}");
    }
    assert_eq!(registrar.holding(), set(&[DEFAULT]));
    assert_eq!(on_disk(dir.path()).hotkeys.check, DEFAULT);
}

// Error path (docs/ARCHITECTURE.md §5, `Invalid {field}`): a value Settings does not know is
// refused naming its field, with the Design notes' message, and nothing changes.
#[test]
fn e_a_setting_with_an_unknown_value_is_invalid() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    let before = api::settings_get(&s);

    let err = api::settings_update(&s, json!({"check": {"dialect": "klingon"}}))
        .expect_err("not one of the five dialects");
    assert_eq!(err, invalid("check.dialect"));
    assert_eq!(api::settings_get(&s), before);
}

// Error path (the Design notes' `settings_update` bounds): `remember_answers_days` at most 7, the
// copy timeout 100 to 5,000 ms and the paste delay 50 to 5,000 ms; a value outside is refused
// naming its field, and the bounds themselves are accepted.
#[test]
fn e_a_setting_out_of_its_range_is_invalid() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    let before = api::settings_get(&s);

    for (patch, field) in [
        (
            json!({"privacy": {"remember_answers_days": 8}}),
            "privacy.remember_answers_days",
        ),
        (
            json!({"capture": {"copy_timeout_ms": 99}}),
            "capture.copy_timeout_ms",
        ),
        (
            json!({"capture": {"copy_timeout_ms": 5_001}}),
            "capture.copy_timeout_ms",
        ),
        (
            json!({"capture": {"paste_restore_delay_ms": 49}}),
            "capture.paste_restore_delay_ms",
        ),
        (
            json!({"capture": {"paste_restore_delay_ms": 5_001}}),
            "capture.paste_restore_delay_ms",
        ),
    ] {
        let err = api::settings_update(&s, patch.clone()).expect_err(&patch.to_string());
        assert_eq!(err, invalid(field), "{patch}");
    }
    assert_eq!(api::settings_get(&s), before);

    let saved = api::settings_update(
        &s,
        json!({"privacy": {"remember_answers_days": 7},
               "capture": {"copy_timeout_ms": 100, "paste_restore_delay_ms": 5_000}}),
    )
    .unwrap_or_else(|e| panic!("the bounds are allowed: {e}"));
    assert_eq!(saved["privacy"]["remember_answers_days"], 7);
    assert_eq!(saved["capture"]["copy_timeout_ms"], 100);
    assert_eq!(saved["capture"]["paste_restore_delay_ms"], 5_000);
}

// Error path (the Design notes' `settings_update`): a settings.json written by a newer Redpen is
// read-only. `settings_get` says so; a change, a new shortcut, Pause and the own key are refused
// with the message, and neither the file nor the registration changes.
#[test]
fn e_settings_written_by_a_newer_redpen_change_nothing() {
    let dir = temp();
    let text = "{\n  \"version\": 2,\n  \"check\": {\"dialect\": \"british\"}\n}\n";
    std::fs::write(dir.path().join("settings.json"), text).expect("settings.json is written");
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    let view = api::settings_get(&s);
    assert_eq!(view["read_only"], true);

    for patch in [
        json!({"check": {"dialect": "american"}}),
        json!({"hotkeys": {"check": FREE}}),
        json!({"ui": {"welcomed": true}}),
    ] {
        let err = api::settings_update(&s, patch.clone()).expect_err(&patch.to_string());
        assert_eq!(err, refused(MSG_NEWER), "{patch}");
    }
    assert_eq!(api::app_pause(&s, true), Err(refused(MSG_NEWER)));
    assert_eq!(
        api::own_key_save(&s, "https://api.example.com/v1", OWN_KEY, OWN_MODEL),
        Err(refused(MSG_NEWER))
    );

    assert_eq!(registrar.holding(), set(&[DEFAULT]));
    assert_eq!(api::settings_get(&s), view);
    let secrets = Secrets::load(dir.path()).expect("secrets.json loads");
    assert!(secrets.own_key.is_none(), "the key was saved");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("settings.json")).expect("settings.json reads"),
        text
    );
}

// The spec's message word for word: Ctrl+Alt+R held by another app when Redpen starts.
#[test]
fn e_the_default_shortcut_taken_at_start_is_named() {
    let registrar = FakeRegistrar::with_others(&[DEFAULT]);
    let err = api::started(DEFAULT, &registrar).expect_err("another app holds Ctrl+Alt+R");
    assert_eq!(kind(&err), "HotkeyInUse");
    assert_eq!(
        message(&err),
        "Ctrl+Alt+R is already used by another app. Choose another shortcut."
    );
    assert!(registrar.holding().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Bullet 13: Pause Redpen.

// Bullet 13: pausing leaves the fake registrar with no hotkeys; unpausing registers the same set
// again.
#[test]
fn t13_pause_unregisters_every_hotkey_and_resume_brings_the_same_set_back() {
    let registrar = FakeRegistrar::default();
    let mut hotkeys = api::started(DEFAULT, &registrar).expect("Ctrl+Alt+R is free");
    let before = registrar.holding();
    assert_eq!(before, set(&[DEFAULT]));

    api::pause(&mut hotkeys, &registrar);
    assert!(
        registrar.holding().is_empty(),
        "{:?}",
        registrar.registered()
    );

    api::resume(&mut hotkeys, &registrar).expect("the same shortcuts register again");
    assert_eq!(registrar.holding(), before);
}

// Bullet 13 through `app_pause`, after a change of shortcut: the paused state is saved, and
// resuming brings back the shortcut in use, not the default.
#[test]
fn t13_app_pause_is_saved_and_resumes_the_shortcut_in_use() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    api::settings_update(&s, json!({"hotkeys": {"check": FREE}})).expect("Ctrl+Alt+J is free");
    let before = registrar.holding();

    api::app_pause(&s, true).expect("Pause Redpen");
    assert!(
        registrar.holding().is_empty(),
        "{:?}",
        registrar.registered()
    );
    assert!(on_disk(dir.path()).ui.paused);
    assert_eq!(api::settings_get(&s)["ui"]["paused"], true);

    api::app_pause(&s, false).expect("unpause");
    assert_eq!(registrar.holding(), before);
    assert!(!on_disk(dir.path()).ui.paused);
}

// Scope (Pause): Redpen started while paused registers nothing until it is unpaused.
#[test]
fn s_a_paused_redpen_starts_with_no_hotkey() {
    let dir = temp();
    let mut paused = Settings::default();
    paused.ui.paused = true;
    paused.save(dir.path()).expect("settings.json is written");

    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    assert!(
        registrar.holding().is_empty(),
        "{:?}",
        registrar.registered()
    );

    api::app_pause(&s, false).expect("unpause");
    assert_eq!(registrar.holding(), set(&[DEFAULT]));
}

// ---------------------------------------------------------------------------------------------
// Bullet 15: Welcome on the first launch only, and nothing sent at launch.

// Bullet 15: the Welcome window opens while `welcomed` is false (a fresh install) and never again
// once it has been closed, across a restart.
#[test]
fn t15_welcome_opens_on_the_first_launch_only() {
    let dir = temp();
    let fresh = on_disk(dir.path());
    assert!(!fresh.ui.welcomed);
    assert!(api::shows_welcome(&fresh));

    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    api::welcome_closed(&s);

    let next_launch = on_disk(dir.path());
    assert!(next_launch.ui.welcomed);
    assert!(!api::shows_welcome(&next_launch));
}

// Bullet 15 and invariant 1: a first launch (no provider chosen, no credential) makes no request;
// the consent view waits for the first AI action.
#[tokio::test]
async fn t15_a_first_launch_sends_nothing() {
    let config = temp();
    let data = temp();
    let t = Arc::new(FakeTransport::scripted(Vec::new()));
    let registrar = Arc::new(FakeRegistrar::default());
    let _s = service(config.path(), &registrar, US);
    assert!(api::shows_welcome(&on_disk(config.path())));

    api::publik_at_start(t.clone(), config.path(), data.path()).await;
    assert!(
        t.requests_seen().is_empty(),
        "a launch sent {} request(s)",
        t.requests_seen().len()
    );
}

// ---------------------------------------------------------------------------------------------
// The own key's address (the spec's Messages).

// Error path: an own key address that is neither https:// nor http on this computer is refused
// with the spec's message; the key is not saved and the error does not carry it.
#[test]
fn e_an_own_key_address_must_be_https_or_this_computer() {
    for refused in [
        "http://api.example.com/v1",
        "ftp://api.example.com/v1",
        "api.example.com/v1",
        "http://localhost.example.com/v1",
    ] {
        let dir = temp();
        let registrar = Arc::new(FakeRegistrar::default());
        let s = service(dir.path(), &registrar, US);
        let err = api::own_key_save(&s, refused, OWN_KEY, OWN_MODEL).expect_err(refused);
        assert_eq!(kind(&err), "InvalidUrl", "{refused}");
        assert_eq!(message(&err), MSG_INVALID_URL, "{refused}");
        assert!(!err.to_string().contains(OWN_KEY), "{refused}: {err}");
        let secrets = Secrets::load(dir.path()).expect("secrets.json loads");
        assert!(secrets.own_key.is_none(), "{refused}: the key was saved");
    }
}

// Scope (own key): an https address or http on this computer is saved; the key goes to
// secrets.json and never comes back in `settings_get` (invariant 6).
#[test]
fn s_an_own_key_on_https_or_localhost_is_saved_and_never_returned() {
    for accepted in [
        "https://api.example.com/v1",
        "http://localhost:11434/v1",
        "http://localhost/v1",
    ] {
        let dir = temp();
        let registrar = Arc::new(FakeRegistrar::default());
        let s = service(dir.path(), &registrar, US);
        api::own_key_save(&s, accepted, OWN_KEY, OWN_MODEL)
            .unwrap_or_else(|e| panic!("{accepted} was refused: {e}"));

        let view = api::settings_get(&s);
        assert_eq!(view["providers"]["own_key"]["base_url"], accepted);
        assert_eq!(view["providers"]["own_key"]["chat_model"], OWN_MODEL);
        assert!(!view.to_string().contains(OWN_KEY), "{accepted}: {view}");
        let secrets = Secrets::load(dir.path()).expect("secrets.json loads");
        assert!(
            secrets.own_key.is_some(),
            "{accepted}: the key was not saved"
        );
    }
}

// Scope (own key, docs/ARCHITECTURE.md §5 `own_key_clear`): clearing forgets the key in
// secrets.json; nothing the interface reads ever held it.
#[test]
fn s_own_key_clear_forgets_the_key() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    api::own_key_save(&s, "https://api.example.com/v1", OWN_KEY, OWN_MODEL)
        .unwrap_or_else(|e| panic!("the own key was refused: {e}"));
    assert!(
        Secrets::load(dir.path())
            .expect("secrets.json loads")
            .own_key
            .is_some()
    );

    api::own_key_clear(&s).expect("the key is forgotten");
    let secrets = Secrets::load(dir.path()).expect("secrets.json loads");
    assert!(secrets.own_key.is_none(), "the key is still saved");
    assert!(!api::settings_get(&s).to_string().contains(OWN_KEY));
}

// Error path (the Design notes' own key): an empty model is `Invalid {field}` naming it, and an
// empty key while none is saved is `Invalid {field: own_key}`; nothing is saved.
#[test]
fn e_an_own_key_needs_a_model_and_a_key() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    let before = api::settings_get(&s);

    for model in ["", "   "] {
        let err = api::own_key_save(&s, "https://api.example.com/v1", OWN_KEY, model)
            .expect_err("there is no model");
        assert_eq!(err, invalid("providers.own_key.chat_model"), "{model:?}");
    }
    for key in ["", "   "] {
        let err = api::own_key_save(&s, "https://api.example.com/v1", key, OWN_MODEL)
            .expect_err("there is no key, and none is saved");
        assert_eq!(err, invalid("own_key"), "{key:?}");
    }

    assert_eq!(api::settings_get(&s), before);
    assert_eq!(before["own_key_saved"], false);
    let secrets = Secrets::load(dir.path()).expect("secrets.json loads");
    assert!(secrets.own_key.is_none(), "a key was saved");
}

// Scope (the Design notes' own key): with a key saved, an empty key keeps it while the address
// and model change.
#[test]
fn s_an_empty_key_keeps_the_saved_one() {
    use redpen_core::provider::ExposeSecret;
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    api::own_key_save(&s, "https://api.example.com/v1", OWN_KEY, OWN_MODEL)
        .unwrap_or_else(|e| panic!("the own key was refused: {e}"));

    api::own_key_save(&s, "http://localhost:8080/v1", "", "example-chat-mini")
        .unwrap_or_else(|e| panic!("an empty key was refused with a key saved: {e}"));
    let view = api::settings_get(&s);
    assert_eq!(view["own_key_saved"], true);
    assert_eq!(
        view["providers"]["own_key"]["base_url"],
        "http://localhost:8080/v1"
    );
    assert_eq!(
        view["providers"]["own_key"]["chat_model"],
        "example-chat-mini"
    );
    let secrets = Secrets::load(dir.path()).expect("secrets.json loads");
    assert_eq!(
        secrets.own_key.as_ref().map(|k| k.expose_secret()),
        Some(OWN_KEY)
    );
}

// Error path (the Design notes' refusals of the settings commands): a secrets.json that cannot be
// written (here a folder stands where the file goes) makes `own_key_save` say "Your key could
// not be saved." and `own_key_clear` "Your key could not be removed."; neither carries the key.
#[test]
fn e_a_key_that_cannot_be_written_is_refused() {
    let dir = temp();
    std::fs::create_dir(dir.path().join("secrets.json")).expect("a folder in the way");
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);

    let err = api::own_key_save(&s, "https://api.example.com/v1", OWN_KEY, OWN_MODEL)
        .expect_err("secrets.json cannot be written");
    assert_eq!(err, refused("Your key could not be saved."));
    assert_eq!(api::settings_get(&s)["own_key_saved"], false);

    let err = api::own_key_clear(&s).expect_err("secrets.json cannot be written");
    assert_eq!(err, refused("Your key could not be removed."));
}

// ---------------------------------------------------------------------------------------------
// The other settings commands (docs/ARCHITECTURE.md §5).

/// A chat client over a fresh in-memory `redpen.db`, whose transport answers nothing, so any
/// request fails the test.
fn ai_client(clock: Arc<FakeClock>) -> (Arc<AiClient>, Arc<FakeTransport>) {
    let t = Arc::new(FakeTransport::scripted(Vec::new()));
    let ai = AiClient::new(
        t.clone(),
        Db::open_in_memory().expect("an in-memory redpen.db"),
        clock,
        Arc::new(FakeSleeper::default()),
        Arc::new(NoopStatusSink),
    );
    (Arc::new(ai), t)
}

/// 2026-12-01T04:30:00Z: day 20,788 after 1970-01-01 (56 years of which 14 leap years to
/// 2026-01-01, then the 334 days of January to November), 4 h 30 min in.
const DEC_1_0430_UTC_MS: i64 = (20_788 * 86_400 + 4 * 3_600 + 30 * 60) * 1_000;

// Error path (the Design notes' refusals): `provider_test` for a kind with nothing to test answers
// `{ok: false}` with the message, and sends nothing (invariant 1).
#[tokio::test]
async fn e_a_provider_with_nothing_to_test_sends_nothing() {
    let dir = temp();
    let registrar = Arc::new(FakeRegistrar::default());
    let s = service(dir.path(), &registrar, US);
    let (ai, t) = ai_client(Arc::new(FakeClock::at(DEC_1_0430_UTC_MS)));

    for kind in ["publik", "fake", "anything"] {
        assert_eq!(
            api::provider_test(&s, &ai, kind).await,
            json!({"ok": false, "message": "There is nothing to test for that provider."}),
            "{kind}"
        );
    }
    assert!(
        t.requests_seen().is_empty(),
        "{} request(s) sent",
        t.requests_seen().len()
    );
}

// Scope (usage; the Design notes' `usage_month`): this local month's `usage_daily` rows for publik
// API, without the Test buttons. At 23:30 on 30 November at UTC−5 (04:30 on 1 December in UTC),
// last month's rows, last year's November, a row filed under 1 December, the own key's and the
// local model's rows and the Test button's row are all left out.
#[test]
fn s_usage_month_sums_this_local_months_publik_rows() {
    let clock = Arc::new(FakeClock::at(DEC_1_0430_UTC_MS));
    clock.set_utc_offset_minutes(-300);
    let (ai, _t) = ai_client(clock.clone());
    for (day, provider, feature, calls, micros) in [
        // Counted.
        ("2026-11-02", "publik", "clearer", 2, 4_200),
        ("2026-11-30", "publik", "clearer", 1, 2_100),
        ("2026-11-30", "publik", "fix-grammar", 3, 630),
        // Left out.
        ("2026-10-31", "publik", "clearer", 5, 10_500),
        ("2025-11-15", "publik", "shorter", 9, 18_900),
        ("2026-12-01", "publik", "clearer", 1, 2_100),
        ("2026-11-30", "own_key", "clearer", 7, 0),
        ("2026-11-30", "local", "shorter", 4, 0),
        ("2026-11-30", "publik", "test", 1, 100),
    ] {
        ai.db()
            .usage_add(day, provider, feature, calls, micros)
            .expect("a usage row is written");
    }

    let usage = api::usage_month(&ai, clock.as_ref());
    // 4,200 + 2,100 + 630 micro-dollars over 2 + 1 + 3 calls.
    assert_eq!(usage["total_micros"], 6_930, "{usage}");
    assert_eq!(usage["calls"], 6, "{usage}");
    let mut features: Vec<Value> = usage["by_feature"]
        .as_array()
        .unwrap_or_else(|| panic!("no by_feature in {usage}"))
        .clone();
    features.sort_by_key(|f| f["feature"].as_str().unwrap_or_default().to_string());
    assert_eq!(
        features,
        [
            json!({"feature": "clearer", "calls": 3, "charge_micros": 6_300}),
            json!({"feature": "fix-grammar", "calls": 3, "charge_micros": 630}),
        ]
    );

    // With nothing counted this month, nothing is shown.
    let (empty, _t) = ai_client(clock.clone());
    assert_eq!(
        api::usage_month(&empty, clock.as_ref()),
        json!({"total_micros": 0, "calls": 0, "by_feature": []})
    );
}

// Error path (the Design notes' refusals): a `redpen.db` that fails makes `sent_log_clear` say
// "The log could not be cleared." and `cache_clear` "The remembered answers could not be
// cleared."; while it works, each empties its table.
#[test]
fn e_a_store_that_fails_is_refused() {
    let (ai, _t) = ai_client(Arc::new(FakeClock::at(DEC_1_0430_UTC_MS)));
    ai.db()
        .sent_log_add(
            DEC_1_0430_UTC_MS,
            "own_key",
            "/chat/completions",
            "clearer",
            OWN_MODEL,
            "Please mail [EMAIL_1] today about the invoice.",
            46,
        )
        .expect("a sent log row is written");
    ai.db()
        .cache_put(&CacheRow {
            key: "k".repeat(64),
            provider: "own_key".into(),
            model: OWN_MODEL.into(),
            prompt_id: "clearer".into(),
            prompt_version: 1,
            response_json: "{\"text\": \"Hi.\"}".into(),
            charge_micros: None,
            created_at_ms: DEC_1_0430_UTC_MS,
        })
        .expect("a cache row is written");

    api::sent_log_clear(&ai).expect("the log is cleared");
    assert!(ai.db().sent_log_list().expect("the log reads").is_empty());
    api::cache_clear(&ai).expect("the cache is cleared");
    assert_eq!(ai.db().cache_len().expect("the cache reads"), 0);

    ai.db()
        .connection()
        .execute_batch("DROP TABLE sent_log; DROP TABLE ai_cache;")
        .expect("the tables are dropped");
    assert_eq!(
        api::sent_log_clear(&ai),
        Err(refused("The log could not be cleared."))
    );
    assert_eq!(
        api::cache_clear(&ai),
        Err(refused("The remembered answers could not be cleared."))
    );
}

// Error path (the Design notes' `open_settings` and refusals): a Settings window that cannot be
// made or shown is "Settings could not be opened."; the window is asked for the tab and section
// by their fixed names, General for a tab it does not know and no section outside Providers.
#[test]
fn e_settings_that_cannot_open_are_refused() {
    let mut asked = Vec::new();
    let err = api::open_settings("providers", Some("own_key"), |target| {
        asked.push(target);
        Err(())
    })
    .expect_err("the window cannot be made");
    assert_eq!(err, refused("Settings could not be opened."));

    for (tab, section) in [("nowhere", Some("local")), ("privacy", Some("local"))] {
        api::open_settings(tab, section, |target| {
            asked.push(target);
            Ok(())
        })
        .unwrap_or_else(|e| panic!("{tab}: {e}"));
    }
    let names: Vec<(Value, Value)> = asked
        .iter()
        .map(|t| (t["tab"].clone(), t["section"].clone()))
        .collect();
    assert_eq!(
        names,
        [
            (json!("providers"), json!("own_key")),
            (json!("general"), Value::Null),
            (json!("privacy"), Value::Null),
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// The check shortcut where selections cannot be read (the Design notes' Linux sentence).

// Error path: where Redpen cannot read a selection yet, a press of the shortcut closes the text
// shown before, reads nothing and says so; where it can, the press opens a session.
#[tokio::test]
async fn e_a_press_where_selections_cannot_be_read_says_so() {
    let clock = Arc::new(FakeClock::at(DEC_1_0430_UTC_MS));
    let (ai, t) = ai_client(clock.clone());
    let capture = Arc::new(FakeCapture::new().with_selection("I went to teh store yesterday."));
    let sessions = Sessions::new(
        capture.clone(),
        ai,
        Arc::new(FixedResolver::failing(AiError::NeedsConsent)),
        clock,
        Arc::new(FakeSleeper::default()),
    );
    let settings = Settings::default();

    let first = api::shortcut(&sessions, &settings, true)
        .await
        .unwrap_or_else(|e| panic!("the selection is read where it can be: {e}"));
    let first_id = first["id"].as_str().expect("a session id").to_string();
    assert!(sessions.is_open(&first_id));
    let reads = capture.calls().len();

    let err = api::shortcut(&sessions, &settings, false)
        .await
        .expect_err("selections cannot be read here");
    assert_eq!(
        err,
        json!({"kind": "CaptureUnavailable", "message": MSG_CAPTURE_UNAVAILABLE})
    );
    assert_eq!(capture.calls().len(), reads, "the press read the window");
    assert!(sessions.is_empty(), "the text shown before is still open");
    assert!(t.requests_seen().is_empty());
}

// ---------------------------------------------------------------------------------------------
// The tray menu (Scope; docs/PRODUCT.md §5).

// Scope: Open Settings, the balance line (disabled) once publik API is set up, Pause Redpen and
// Quit, Quit last; before publik API is set up there is no balance item.
#[test]
fn s_the_tray_menu() {
    let line = "$3.12 of publik balance";
    let menu = api::tray_menu(Some(line));
    let labels: Vec<&str> = menu.iter().map(|(label, _)| label.as_str()).collect();
    let mut sorted = labels.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        [
            "$3.12 of publik balance",
            "Open Settings",
            "Pause Redpen",
            "Quit"
        ]
    );
    assert_eq!(labels.last(), Some(&"Quit"));
    let at = |label: &str| labels.iter().position(|l| *l == label).unwrap();
    assert!(at("Open Settings") < at("Pause Redpen"), "{labels:?}");
    for (label, enabled) in &menu {
        assert_eq!(*enabled, label != line, "{label}");
    }

    let menu = api::tray_menu(None);
    let labels: Vec<&str> = menu.iter().map(|(label, _)| label.as_str()).collect();
    assert_eq!(labels, ["Open Settings", "Pause Redpen", "Quit"]);
    assert!(menu.iter().all(|(_, enabled)| *enabled));
}

// ---------------------------------------------------------------------------------------------
// Bullet 17: the popup latency check.

/// Whether the desktop tests were asked for: they type into the window in front and replace the
/// clipboard (docs/TESTING.md §1).
fn desktop_tests() -> bool {
    std::env::var_os("REDPEN_DESKTOP_TESTS").as_deref() == Some(OsStr::new("1"))
}

// Bullet 17: `redpen-app --measure-popup` with `redpen-testwin` reports p95 at or under 300 ms
// from hotkey to findings over 20 runs.
#[test]
fn t17_the_popup_has_its_findings_within_300_ms_p95() {
    if !cfg!(windows) || !desktop_tests() {
        eprintln!("skipped: set REDPEN_DESKTOP_TESTS=1 on Windows, with nothing else open");
        return;
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_redpen-app"))
        .arg("--measure-popup")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("redpen-app starts");
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if child.try_wait().expect("the run can be watched").is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("--measure-popup did not finish within 180 seconds");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let out = child.wait_with_output().expect("the output is read");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "--measure-popup failed: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("no JSON line in: {stdout}"));
    let report: Value =
        serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("{line:?} is not JSON: {e}"));
    let p95 = report["p95_ms"]
        .as_f64()
        .unwrap_or_else(|| panic!("no p95_ms in {report}"));
    assert!(p95 > 0.0, "{report}");
    assert!(p95 <= 300.0, "p95 {p95} ms is over 300 ms: {report}");
}
