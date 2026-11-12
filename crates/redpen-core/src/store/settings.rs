//! `settings.json` (docs/ARCHITECTURE.md §4): the full schema with its defaults. A missing field
//! takes its default; a field this version does not know, at any depth, is kept when the file is
//! written again; a file written by a newer Redpen (`version` above 1) opens read-only.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::check::{Category, Dialect, dialect_from_locale};

/// The settings version this Redpen reads and writes.
pub const SETTINGS_VERSION: u32 = 1;

/// The processes the capture refuses by default (password managers).
pub const DEFAULT_BLOCKLIST: [&str; 8] = [
    "1password.exe",
    "bitwarden.exe",
    "keepass.exe",
    "keepassxc.exe",
    "dashlane.exe",
    "enpass.exe",
    "nordpass.exe",
    "proton pass.exe",
];

/// Every setting. `Default` is the schema's defaults with American English; the app starts
/// from [`Settings::for_locale`] when there is no file yet.
///
/// [`Settings::save`] writes back every field of the file this version does not know. A value
/// loaded from a file written by a newer Redpen is read-only ([`Settings::read_only`]).
/// Equality compares the settings only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub hotkeys: Hotkeys,
    pub check: CheckSettings,
    pub providers: ProviderSettings,
    pub privacy: PrivacySettings,
    pub capture: CaptureSettings,
    pub plagiarism: PlagiarismSettings,
    pub ui: UiSettings,
    /// The file's `version` is newer than this Redpen's.
    #[serde(skip)]
    read_only: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            hotkeys: Hotkeys::default(),
            check: CheckSettings::default(),
            providers: ProviderSettings::default(),
            privacy: PrivacySettings::default(),
            capture: CaptureSettings::default(),
            plagiarism: PlagiarismSettings::default(),
            ui: UiSettings::default(),
            read_only: false,
        }
    }
}

impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && self.hotkeys == other.hotkeys
            && self.check == other.check
            && self.providers == other.providers
            && self.privacy == other.privacy
            && self.capture == other.capture
            && self.plagiarism == other.plagiarism
            && self.ui == other.ui
    }
}

impl Eq for Settings {}

impl Settings {
    /// The file's name in the config directory.
    pub const FILE: &'static str = "settings.json";

    /// The defaults, with the dialect taken from the operating system's locale (en-GB British,
    /// en-US American, en-AU Australian, en-CA Canadian, en-IN Indian, anything else American).
    pub fn for_locale(locale: Option<&str>) -> Self {
        let mut s = Self::default();
        s.check.dialect = locale.map(dialect_from_locale).unwrap_or_default();
        s
    }

    /// Reads `<config_dir>/settings.json`; a missing file gives the defaults (American English),
    /// and is not written until [`Settings::save`].
    pub fn load(config_dir: &Path) -> Result<Self, SettingsError> {
        Self::load_for_locale(config_dir, None)
    }

    /// As [`Settings::load`], with a missing file giving the defaults for `locale`. A file with
    /// a newer `version` loads read-only, its known fields read as well as they can be.
    pub fn load_for_locale(config_dir: &Path, locale: Option<&str>) -> Result<Self, SettingsError> {
        let path = config_dir.join(Self::FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::for_locale(locale));
            }
            Err(e) => return Err(SettingsError::Io(e.kind().to_string())),
        };
        let raw: Value =
            serde_json::from_str(&text).map_err(|e| SettingsError::Invalid(e.to_string()))?;
        if !raw.is_object() {
            return Err(SettingsError::Invalid(
                "the file is not a JSON object".into(),
            ));
        }
        let newer = raw
            .get("version")
            .and_then(Value::as_u64)
            .is_some_and(|v| v > u64::from(SETTINGS_VERSION));
        let mut settings = match serde_json::from_value::<Settings>(raw.clone()) {
            Ok(s) => s,
            // A newer schema may have changed a field's type; read-only, the defaults will do.
            Err(_) if newer => Self::for_locale(locale),
            Err(e) => return Err(SettingsError::Invalid(e.to_string())),
        };
        settings.read_only = newer;
        Ok(settings)
    }

    /// Whether the file was written by a newer Redpen, so [`Settings::save`] is refused.
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// Writes `<config_dir>/settings.json` atomically (a temporary file, then a rename), as
    /// version 1, over the file as it is on disk now, so every field this version does not know
    /// is kept. Refused with [`SettingsError::NewerVersion`] when these settings, or the file
    /// on disk, came from a newer Redpen; the file is then left as it is.
    pub fn save(&self, config_dir: &Path) -> Result<(), SettingsError> {
        let path = config_dir.join(Self::FILE);
        let on_disk = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .filter(Value::is_object);
        let newer_on_disk = on_disk
            .as_ref()
            .and_then(|v| v.get("version"))
            .and_then(Value::as_u64)
            .is_some_and(|v| v > u64::from(SETTINGS_VERSION));
        if self.read_only || newer_on_disk {
            return Err(SettingsError::NewerVersion);
        }
        let mut known = serde_json::to_value(self).expect("settings serialise");
        known["version"] = Value::from(SETTINGS_VERSION);
        let mut out = on_disk.unwrap_or_else(|| Value::Object(Default::default()));
        merge(&mut out, known);
        let mut text = serde_json::to_string_pretty(&out).expect("JSON serialises");
        text.push('\n');
        super::write_atomic(&path, text.as_bytes(), false)
            .map_err(|e| SettingsError::Io(e.kind().to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub check: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            check: "Ctrl+Alt+R".into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckSettings {
    pub dialect: Dialect,
    pub disabled_categories: Vec<Category>,
}

/// Settings → Providers → AI provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatChoice {
    Publik,
    OwnKey,
    Local,
}

/// Settings → Providers → Web search (module 09).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchChoice {
    Publik,
    OwnKey,
}

/// Settings → Providers → Rewrite quality.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RewriteQuality {
    #[default]
    Standard,
    Best,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSettings {
    /// `None` until the user chooses.
    pub chat: Option<ChatChoice>,
    pub rewrite_quality: RewriteQuality,
    /// `None` until the user chooses (module 09).
    pub search: Option<SearchChoice>,
    pub own_key: OwnKeySettings,
    pub local: LocalSettings,
}

/// The own key's address and model. The key itself is in `secrets.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OwnKeySettings {
    pub base_url: String,
    pub chat_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalSettings {
    pub chat_base_url: String,
    /// `None` means "pick the largest suitable model Ollama has".
    pub chat_model: Option<String>,
}

impl Default for LocalSettings {
    fn default() -> Self {
        Self {
            chat_base_url: crate::provider::local::DEFAULT_LOCAL_BASE_URL.into(),
            chat_model: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacySettings {
    pub redaction: bool,
    /// How long rewrite answers stay in `ai_cache`, at most 7; 0 keeps them in memory only.
    pub remember_answers_days: u32,
    /// Settings → Privacy → "Keep a log".
    pub sent_log: bool,
    pub blocklist: Vec<String>,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            redaction: true,
            remember_answers_days: 7,
            sent_log: false,
            blocklist: DEFAULT_BLOCKLIST.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureSettings {
    pub copy_timeout_ms: u32,
    pub paste_restore_delay_ms: u32,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            copy_timeout_ms: 400,
            paste_restore_delay_ms: 250,
        }
    }
}

/// The citation style of the plagiarism report (module 09).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CitationStyle {
    #[default]
    Apa7,
    Mla9,
    Chicago18,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlagiarismSettings {
    pub citation_style: CitationStyle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub start_with_os: bool,
    pub update_check: bool,
    pub paused: bool,
    pub welcomed: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            start_with_os: false,
            update_check: true,
            paused: false,
            welcomed: false,
        }
    }
}

/// Why settings could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// The file's `version` is above 1: it stays as it is.
    #[error("Settings were written by a newer Redpen.")]
    NewerVersion,
    #[error("settings.json could not be read or written: {0}")]
    Io(String),
    #[error("settings.json is not valid: {0}")]
    Invalid(String),
}

/// Writes every field of `known` into `out`, recursing into objects, so fields `out` has and
/// `known` does not are kept.
fn merge(out: &mut Value, known: Value) {
    match (out, known) {
        (Value::Object(out), Value::Object(known)) => {
            for (key, value) in known {
                match out.get_mut(&key) {
                    Some(existing) if existing.is_object() && value.is_object() => {
                        merge(existing, value)
                    }
                    _ => {
                        out.insert(key, value);
                    }
                }
            }
        }
        (out, known) => *out = known,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The complete example of docs/ARCHITECTURE.md §4, with the own key's fields filled in.
    fn architecture_example() -> Value {
        json!({
          "version": 1,
          "hotkeys": {"check": "Ctrl+Alt+R"},
          "check": {"dialect": "british", "disabled_categories": []},
          "providers": {"chat": null, "rewrite_quality": "standard", "search": null,
            "own_key": {"base_url": "https://api.example.com/v1", "chat_model": "example-chat"},
            "local": {"chat_base_url": "http://localhost:11434/v1", "chat_model": null}},
          "privacy": {"redaction": true, "remember_answers_days": 7, "sent_log": false,
                      "blocklist": ["1password.exe", "bitwarden.exe", "keepass.exe", "keepassxc.exe",
                                    "dashlane.exe", "enpass.exe", "nordpass.exe", "proton pass.exe"]},
          "capture": {"copy_timeout_ms": 400, "paste_restore_delay_ms": 250},
          "plagiarism": {"citation_style": "apa7"},
          "ui": {"start_with_os": false, "update_check": true, "paused": false, "welcomed": false}
        })
    }

    #[test]
    fn defaults_are_the_architecture_example() {
        let mut expected = architecture_example();
        expected["providers"]["own_key"] = json!({"base_url": "", "chat_model": ""});
        assert_eq!(
            serde_json::to_value(Settings::for_locale(Some("en-GB"))).unwrap(),
            expected
        );
        assert_eq!(Settings::default().check.dialect, Dialect::American);
        assert_eq!(
            Settings::for_locale(Some("fr-FR")).check.dialect,
            Dialect::American
        );
    }

    #[test]
    fn the_example_parses_and_missing_fields_take_defaults() {
        let s: Settings = serde_json::from_value(architecture_example()).unwrap();
        assert_eq!(s.providers.own_key.chat_model, "example-chat");
        assert_eq!(s.check.dialect, Dialect::British);
        let partial: Settings =
            serde_json::from_value(json!({"privacy": {"sent_log": true}})).unwrap();
        assert!(partial.privacy.sent_log);
        assert_eq!(partial.privacy.remember_answers_days, 7);
        assert!(partial.privacy.redaction);
        assert_eq!(partial.hotkeys.check, "Ctrl+Alt+R");
    }

    #[test]
    fn a_missing_file_gives_the_defaults_and_save_writes_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::load_for_locale(dir.path(), Some("en-AU")).unwrap();
        assert!(!s.read_only());
        assert_eq!(s.check.dialect, Dialect::Australian);
        assert_eq!(Settings::load(dir.path()).unwrap(), Settings::default());
        assert!(!dir.path().join("settings.json").exists());
        s.providers.chat = Some(ChatChoice::Local);
        s.save(dir.path()).unwrap();
        let again = Settings::load(dir.path()).unwrap();
        assert_eq!(again.providers.chat, Some(ChatChoice::Local));
        assert_eq!(again.check.dialect, Dialect::Australian);
        assert_eq!(again, s);
    }

    #[test]
    fn unknown_fields_survive_a_save_at_every_depth() {
        let dir = tempfile::tempdir().unwrap();
        let mut original = architecture_example();
        original["future"] = json!({"x": 1});
        original["privacy"]["later"] = json!([1, 2]);
        std::fs::write(
            dir.path().join("settings.json"),
            serde_json::to_string(&original).unwrap(),
        )
        .unwrap();
        let mut s = Settings::load(dir.path()).unwrap();
        s.privacy.sent_log = true;
        s.save(dir.path()).unwrap();
        // Settings built from scratch keep the file's unknown fields too.
        Settings::default().save(dir.path()).unwrap();
        let mut s = Settings::load(dir.path()).unwrap();
        s.privacy.sent_log = true;
        s.save(dir.path()).unwrap();
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written["future"], json!({"x": 1}));
        assert_eq!(written["privacy"]["later"], json!([1, 2]));
        assert_eq!(written["privacy"]["sent_log"], json!(true));
        assert_eq!(written["version"], json!(1));
    }

    #[test]
    fn a_newer_version_is_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let text = r#"{"version": 2, "privacy": {"sent_log": true}, "hotkeys": "changed shape"}"#;
        std::fs::write(dir.path().join("settings.json"), text).unwrap();
        let s = Settings::load(dir.path()).unwrap();
        assert!(s.read_only());
        let err = s.save(dir.path()).unwrap_err();
        assert_eq!(err, SettingsError::NewerVersion);
        assert_eq!(err.to_string(), "Settings were written by a newer Redpen.");
        assert!(s.clone().save(dir.path()).is_err());
        // Settings that were not loaded from it cannot overwrite it either.
        assert_eq!(
            Settings::default().save(dir.path()),
            Err(SettingsError::NewerVersion)
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
            text
        );
        let parsed = r#"{"version": 2, "privacy": {"sent_log": true}}"#;
        std::fs::write(dir.path().join("settings.json"), parsed).unwrap();
        let s = Settings::load(dir.path()).unwrap();
        assert!(s.read_only());
        assert!(s.privacy.sent_log);
    }

    #[test]
    fn a_broken_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{not json").unwrap();
        assert!(matches!(
            Settings::load(dir.path()),
            Err(SettingsError::Invalid(_))
        ));
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"privacy": {"remember_answers_days": "seven"}}"#,
        )
        .unwrap();
        assert!(matches!(
            Settings::load(dir.path()),
            Err(SettingsError::Invalid(_))
        ));
    }
}
