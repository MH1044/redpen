//! `secrets.json` (docs/ARCHITECTURE.md §4): `{"version": 1, "own_key": "…", "search_key": "…"}`,
//! either key absent when not saved. It lives in the per-user config folder, written with mode
//! 0600 on Unix; on Windows the folder's access list keeps other users out, and there is no
//! DPAPI in v1 (the Publik credential is plain JSON by contract, and the two match).
//!
//! The keys are [`SecretString`]s from the moment they are read: `Debug` prints `***`, and no
//! error message carries any part of the file (invariant 6).

use std::fmt;
use std::path::Path;

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value};

/// The keys the user saved.
#[derive(Clone, Default)]
pub struct Secrets {
    /// The key for Settings → Providers → Your own key.
    pub own_key: Option<SecretString>,
    /// The Brave Search API key for plagiarism checks (module 09).
    pub search_key: Option<SecretString>,
}

impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secrets")
            .field("own_key", &self.own_key.as_ref().map(|_| "***"))
            .field("search_key", &self.search_key.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Why `secrets.json` could not be read or written. The messages never quote the file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretsError {
    #[error("secrets.json could not be read or written: {0}")]
    Io(String),
    #[error("secrets.json is damaged. Save your key again in Settings → Providers.")]
    Invalid,
}

impl Secrets {
    /// The file's name in the config directory.
    pub const FILE: &'static str = "secrets.json";

    /// Reads `<config_dir>/secrets.json`; a missing file means no keys are saved. An empty
    /// string counts as no key.
    pub fn load(config_dir: &Path) -> Result<Self, SecretsError> {
        let text = match std::fs::read_to_string(config_dir.join(Self::FILE)) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(SecretsError::Io(io_reason(&e))),
        };
        // serde_json's messages can quote a value, so its error is dropped, never shown.
        let value: Value = serde_json::from_str(&text).map_err(|_| SecretsError::Invalid)?;
        let object = value.as_object().ok_or(SecretsError::Invalid)?;
        let key = |name: &str| -> Result<Option<SecretString>, SecretsError> {
            match object.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) if s.is_empty() => Ok(None),
                Some(Value::String(s)) => Ok(Some(SecretString::from(s.as_str()))),
                Some(_) => Err(SecretsError::Invalid),
            }
        };
        Ok(Self {
            own_key: key("own_key")?,
            search_key: key("search_key")?,
        })
    }

    /// Writes `<config_dir>/secrets.json` atomically, mode 0600 on Unix.
    pub fn save(&self, config_dir: &Path) -> Result<(), SecretsError> {
        let mut object = Map::new();
        object.insert("version".into(), Value::from(1));
        if let Some(k) = &self.own_key {
            object.insert("own_key".into(), Value::from(k.expose_secret()));
        }
        if let Some(k) = &self.search_key {
            object.insert("search_key".into(), Value::from(k.expose_secret()));
        }
        let mut text =
            serde_json::to_string_pretty(&Value::Object(object)).expect("JSON serialises");
        text.push('\n');
        super::write_atomic(&config_dir.join(Self::FILE), text.as_bytes(), true)
            .map_err(|e| SecretsError::Io(io_reason(&e)))
    }
}

/// The operating system's reason, without a path.
fn io_reason(e: &std::io::Error) -> String {
    e.kind().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_then_loading_returns_the_keys() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Secrets {
            own_key: Some(SecretString::from("sk-test-a")),
            search_key: None,
        };
        secrets.save(dir.path()).unwrap();
        let back = Secrets::load(dir.path()).unwrap();
        assert_eq!(back.own_key.unwrap().expose_secret(), "sk-test-a");
        assert!(back.search_key.is_none());
        let text = std::fs::read_to_string(dir.path().join("secrets.json")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"version": 1, "own_key": "sk-test-a"})
        );
    }

    #[test]
    fn a_missing_file_has_no_keys() {
        let dir = tempfile::tempdir().unwrap();
        let s = Secrets::load(dir.path()).unwrap();
        assert!(s.own_key.is_none() && s.search_key.is_none());
    }

    #[test]
    fn debug_and_errors_never_show_a_key() {
        let s = Secrets {
            own_key: Some(SecretString::from("sk-test-a")),
            search_key: Some(SecretString::from("brave-test-b")),
        };
        let debug = format!("{s:?}");
        assert!(
            !debug.contains("sk-test-a") && !debug.contains("brave-test-b"),
            "{debug}"
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("secrets.json"),
            r#"{"version": "sk-test-a", "own_key": 7}"#,
        )
        .unwrap();
        let err = Secrets::load(dir.path()).unwrap_err();
        assert_eq!(err, SecretsError::Invalid);
        assert!(!err.to_string().contains("sk-test-a"));
        std::fs::write(dir.path().join("secrets.json"), "sk-test-a").unwrap();
        let err = Secrets::load(dir.path()).unwrap_err();
        assert!(!format!("{err} {err:?}").contains("sk-test-a"));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        Secrets {
            own_key: Some(SecretString::from("sk-test-a")),
            search_key: None,
        }
        .save(dir.path())
        .unwrap();
        let mode = std::fs::metadata(dir.path().join("secrets.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
