//! The provider a call goes to, once resolved: its kind, address, key and chat model
//! (docs/modules/02-provider-layer.md). Module 03 resolves publik API into the same shape.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use super::errors::AiError;
use crate::store::secrets::Secrets;
use crate::store::settings::{LocalSettings, OwnKeySettings};

/// Which provider answers. The names (`publik`, `own_key`, `local`, `fake`) are the ones
/// `ai_cache.provider`, `usage_daily.provider` and `sent_log.provider` hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Publik,
    OwnKey,
    Local,
    Fake,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Publik => "publik",
            ProviderKind::OwnKey => "own_key",
            ProviderKind::Local => "local",
            ProviderKind::Fake => "fake",
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A provider ready for a call. The key is a [`SecretString`]: it is sent only as
/// `Authorization: Bearer`, and `Debug` prints `***` in its place (invariant 6).
#[derive(Clone)]
pub struct ResolvedProvider {
    pub kind: ProviderKind,
    pub base_url: String,
    pub key: Option<SecretString>,
    pub chat_model: String,
}

impl fmt::Debug for ResolvedProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedProvider")
            .field("kind", &self.kind)
            .field("base_url", &self.base_url)
            .field("key", &self.key.as_ref().map(|_| "***"))
            .field("chat_model", &self.chat_model)
            .finish()
    }
}

impl ResolvedProvider {
    /// The fake provider's address: `.invalid` never resolves, so it cannot reach a network.
    pub const FAKE_BASE_URL: &'static str = "http://fake.invalid/v1";
    /// The fake provider's chat model.
    pub const FAKE_MODEL: &'static str = "fake-chat";

    /// The fake provider: kind `Fake`, `http://fake.invalid/v1`, model `fake-chat`, no key.
    pub fn fake() -> Self {
        Self {
            kind: ProviderKind::Fake,
            base_url: Self::FAKE_BASE_URL.to_string(),
            key: None,
            chat_model: Self::FAKE_MODEL.to_string(),
        }
    }

    /// The user's own key: any OpenAI-compatible address, key and chat model.
    pub fn own_key(
        base_url: impl Into<String>,
        key: impl Into<SecretString>,
        chat_model: impl Into<String>,
    ) -> Self {
        Self {
            kind: ProviderKind::OwnKey,
            base_url: base_url.into(),
            key: Some(key.into()),
            chat_model: chat_model.into(),
        }
    }

    /// A local server. An empty `chat_model` means "ask Ollama and pick one"
    /// ([`super::local::pick_local_model`]).
    pub fn local(base_url: impl Into<String>, chat_model: impl Into<String>) -> Self {
        Self {
            kind: ProviderKind::Local,
            base_url: base_url.into(),
            key: None,
            chat_model: chat_model.into(),
        }
    }

    /// The own key as saved: the base URL and chat model from Settings → Providers → Your own
    /// key (`settings.json`), the key from `secrets.json`. `NotConfigured` when any of the three
    /// is empty.
    pub fn from_own_key_settings(
        settings: &OwnKeySettings,
        secrets: &Secrets,
    ) -> Result<Self, AiError> {
        let key = secrets
            .own_key
            .as_ref()
            .filter(|k| !k.expose_secret().trim().is_empty())
            .ok_or(AiError::NotConfigured)?;
        let base_url = settings.base_url.trim();
        let chat_model = settings.chat_model.trim();
        if base_url.is_empty() || chat_model.is_empty() {
            return Err(AiError::NotConfigured);
        }
        Ok(Self {
            kind: ProviderKind::OwnKey,
            base_url: base_url.to_string(),
            key: Some(key.clone()),
            chat_model: chat_model.to_string(),
        })
    }

    /// The local provider as Settings → Providers → Local describes it; no model chosen means
    /// Ollama's pick at the time of the call.
    pub fn from_local_settings(settings: &LocalSettings) -> Self {
        Self::local(
            settings.chat_base_url.trim(),
            settings.chat_model.as_deref().unwrap_or("").trim(),
        )
    }

    /// `path` (starting with `/`) under the base URL, without a doubled slash.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_key() {
        let p =
            ResolvedProvider::own_key("https://api.example.com/v1", "sk-test-a", "example-chat");
        let debug = format!("{p:?}");
        assert!(!debug.contains("sk-test-a"), "{debug}");
        assert!(debug.contains("***"), "{debug}");
        let debug = format!("{:#?}", p.clone());
        assert!(!debug.contains("sk-test-a"), "{debug}");
    }

    #[test]
    fn the_fake_provider_has_no_key() {
        let p = ResolvedProvider::fake();
        assert_eq!(p.kind, ProviderKind::Fake);
        assert_eq!(p.base_url, "http://fake.invalid/v1");
        assert_eq!(p.chat_model, "fake-chat");
        assert!(p.key.is_none());
    }

    #[test]
    fn urls_join_without_a_doubled_slash() {
        let p = ResolvedProvider::local("http://localhost:11434/v1/", "m");
        assert_eq!(
            p.url("/chat/completions"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            ResolvedProvider::fake().url("/models"),
            "http://fake.invalid/v1/models"
        );
    }

    #[test]
    fn the_own_key_comes_from_settings_and_secrets() {
        let settings = OwnKeySettings {
            base_url: "https://api.example.com/v1".into(),
            chat_model: "example-chat".into(),
        };
        let secrets = Secrets {
            own_key: Some(SecretString::from("sk-test-a")),
            search_key: None,
        };
        let p = ResolvedProvider::from_own_key_settings(&settings, &secrets).unwrap();
        assert_eq!(p.kind, ProviderKind::OwnKey);
        assert_eq!(p.base_url, "https://api.example.com/v1");
        assert_eq!(p.chat_model, "example-chat");
        assert_eq!(p.key.unwrap().expose_secret(), "sk-test-a");
        assert_eq!(
            ResolvedProvider::from_own_key_settings(&settings, &Secrets::default()).unwrap_err(),
            AiError::NotConfigured
        );
        assert_eq!(
            ResolvedProvider::from_own_key_settings(&OwnKeySettings::default(), &secrets)
                .unwrap_err(),
            AiError::NotConfigured
        );
    }

    #[test]
    fn the_local_provider_comes_from_settings() {
        let p = ResolvedProvider::from_local_settings(&LocalSettings::default());
        assert_eq!(p.kind, ProviderKind::Local);
        assert_eq!(p.base_url, "http://localhost:11434/v1");
        assert_eq!(p.chat_model, "");
        let chosen = LocalSettings {
            chat_base_url: "http://127.0.0.1:1234/v1".into(),
            chat_model: Some("qwen2.5:14b".into()),
        };
        let p = ResolvedProvider::from_local_settings(&chosen);
        assert_eq!(
            (p.base_url.as_str(), p.chat_model.as_str()),
            ("http://127.0.0.1:1234/v1", "qwen2.5:14b")
        );
    }

    #[test]
    fn kinds_have_their_database_names() {
        let names: Vec<&str> = [
            ProviderKind::Publik,
            ProviderKind::OwnKey,
            ProviderKind::Local,
            ProviderKind::Fake,
        ]
        .iter()
        .map(ProviderKind::as_str)
        .collect();
        assert_eq!(names, ["publik", "own_key", "local", "fake"]);
        assert_eq!(
            serde_json::to_value(ProviderKind::OwnKey).unwrap(),
            serde_json::json!("own_key")
        );
    }
}
