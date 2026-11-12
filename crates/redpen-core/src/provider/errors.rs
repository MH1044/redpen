//! What a provider call can fail with, and the message a user sees for each
//! (docs/modules/02-provider-layer.md, Interfaces). No variant carries a key, so neither the
//! messages nor `Debug` can ever show one (invariant 6).

use serde::Serialize;

use super::resolved::ProviderKind;

/// The message of [`AiError::RateLimited`] on the user's own key (and on a local server).
pub const OWN_KEY_RATE_LIMITED: &str =
    "Your provider says you are sending too many requests (429). Wait a moment and try again.";

/// A provider call that did not give a usable answer. `Display` is the message the interface
/// shows; the interface receives it as `{"kind", "message"}` plus the variant's fields.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    /// No provider is chosen, or the own key is missing its key, address or model.
    #[error("Choose an AI provider in Settings → Providers.")]
    NotConfigured,
    /// 401 or 403 from the provider.
    #[error("Your key was refused ({code}). Check it in Settings → Providers.")]
    OwnKeyRefused { code: u16 },
    /// 404 on a chat call: the provider has no such model.
    #[error(
        "Your provider does not know the model \u{201c}{model}\u{201d} (404). Check the model name in Settings."
    )]
    ModelUnknown { model: String },
    /// 429. `message` is what the user sees ([`OWN_KEY_RATE_LIMITED`] for the own key).
    #[error("{message}")]
    RateLimited { message: String },
    /// A 5xx, or a 4xx with no message of its own.
    #[error("Your provider had a problem ({code}). Try again later.")]
    ProviderError { code: u16 },
    /// No answer within the call's timeout.
    #[error("No answer from {} after {seconds} seconds. Try again.", who_phrase(.who))]
    Timeout { who: ProviderKind, seconds: u64 },
    /// No connection could be made (or the TLS handshake failed).
    #[error("No connection. Rewrites need a connection or a local model; checking still works.")]
    Offline,
    /// Nothing answers at the local address.
    #[error("Ollama isn't running. Start it, or choose another provider.")]
    LocalNotRunning,
    /// Ollama answers but has no chat model Redpen can use.
    #[error(
        "Ollama has no chat model installed. Install one in Ollama, or choose another provider."
    )]
    LocalNoModel,
    /// The local provider's address is not this computer.
    #[error("A local model must run on this computer (localhost).")]
    LocalNotLocal,
    /// The answer is not JSON, or has no `choices[0].message.content` string.
    #[error("The AI answer could not be used.")]
    InvalidAnswer,
    /// The fake transport had no entry for the request (docs/TESTING.md §1). Only tests and
    /// the command line's `--recording` see it.
    #[error(
        "Unrecorded request {hash} to {path}; body written to target/unrecorded/{hash}.json. Add a hand-written entry, or ask the maintainer to record it."
    )]
    Unrecorded { hash: String, path: String },
}

/// How the timeout message names the provider.
fn who_phrase(kind: &ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OwnKey => "your own key's server",
        ProviderKind::Local => "the local model",
        ProviderKind::Publik => "publik API",
        ProviderKind::Fake => "the fake provider",
    }
}

impl AiError {
    /// The rate-limit error of the own key and the local provider.
    pub fn rate_limited() -> Self {
        AiError::RateLimited {
            message: OWN_KEY_RATE_LIMITED.to_string(),
        }
    }

    /// The variant's name, as the interface and the command line report it.
    pub fn kind(&self) -> &'static str {
        match self {
            AiError::NotConfigured => "NotConfigured",
            AiError::OwnKeyRefused { .. } => "OwnKeyRefused",
            AiError::ModelUnknown { .. } => "ModelUnknown",
            AiError::RateLimited { .. } => "RateLimited",
            AiError::ProviderError { .. } => "ProviderError",
            AiError::Timeout { .. } => "Timeout",
            AiError::Offline => "Offline",
            AiError::LocalNotRunning => "LocalNotRunning",
            AiError::LocalNoModel => "LocalNoModel",
            AiError::LocalNotLocal => "LocalNotLocal",
            AiError::InvalidAnswer => "InvalidAnswer",
            AiError::Unrecorded { .. } => "Unrecorded",
        }
    }

    /// `{"kind", "message"}` plus the variant's fields.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("an error serialises")
    }
}

/// Serialised as `{"kind", "message"}`, then the variant's fields (`code`, `model`, `who` and
/// `seconds`, `hash` and `path`). `RateLimited`'s field is the message itself.
impl Serialize for AiError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("kind", self.kind())?;
        map.serialize_entry("message", &self.to_string())?;
        match self {
            AiError::OwnKeyRefused { code } | AiError::ProviderError { code } => {
                map.serialize_entry("code", code)?;
            }
            AiError::ModelUnknown { model } => map.serialize_entry("model", model)?,
            AiError::Timeout { who, seconds } => {
                map.serialize_entry("who", who)?;
                map.serialize_entry("seconds", seconds)?;
            }
            AiError::Unrecorded { hash, path } => {
                map.serialize_entry("hash", hash)?;
                map.serialize_entry("path", path)?;
            }
            _ => {}
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_are_the_spec_strings() {
        let cases = [
            (
                AiError::NotConfigured,
                "Choose an AI provider in Settings → Providers.",
            ),
            (
                AiError::OwnKeyRefused { code: 401 },
                "Your key was refused (401). Check it in Settings → Providers.",
            ),
            (
                AiError::ModelUnknown {
                    model: "example-chat".into(),
                },
                "Your provider does not know the model “example-chat” (404). Check the model name in Settings.",
            ),
            (
                AiError::rate_limited(),
                "Your provider says you are sending too many requests (429). Wait a moment and try again.",
            ),
            (
                AiError::ProviderError { code: 502 },
                "Your provider had a problem (502). Try again later.",
            ),
            (
                AiError::Timeout {
                    who: ProviderKind::OwnKey,
                    seconds: 30,
                },
                "No answer from your own key's server after 30 seconds. Try again.",
            ),
            (
                AiError::Timeout {
                    who: ProviderKind::Local,
                    seconds: 120,
                },
                "No answer from the local model after 120 seconds. Try again.",
            ),
            (
                AiError::Offline,
                "No connection. Rewrites need a connection or a local model; checking still works.",
            ),
            (
                AiError::LocalNotRunning,
                "Ollama isn't running. Start it, or choose another provider.",
            ),
            (
                AiError::LocalNoModel,
                "Ollama has no chat model installed. Install one in Ollama, or choose another provider.",
            ),
            (
                AiError::LocalNotLocal,
                "A local model must run on this computer (localhost).",
            ),
            (AiError::InvalidAnswer, "The AI answer could not be used."),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message, "{error:?}");
        }
    }

    #[test]
    fn unrecorded_carries_the_transport_message() {
        let error = AiError::Unrecorded {
            hash: "abc".into(),
            path: "/chat/completions".into(),
        };
        let transport = crate::net::TransportError::Unrecorded {
            hash: "abc".into(),
            path: "/chat/completions".into(),
        };
        assert_eq!(error.to_string(), transport.to_string());
    }

    #[test]
    fn serialises_kind_message_and_fields() {
        assert_eq!(
            AiError::OwnKeyRefused { code: 403 }.to_json(),
            json!({"kind": "OwnKeyRefused", "code": 403,
                   "message": "Your key was refused (403). Check it in Settings → Providers."})
        );
        assert_eq!(
            AiError::Timeout {
                who: ProviderKind::Local,
                seconds: 120
            }
            .to_json(),
            json!({"kind": "Timeout", "who": "local", "seconds": 120,
                   "message": "No answer from the local model after 120 seconds. Try again."})
        );
        assert_eq!(
            AiError::Offline.to_json(),
            json!({"kind": "Offline",
                   "message": "No connection. Rewrites need a connection or a local model; checking still works."})
        );
        assert_eq!(
            AiError::rate_limited().to_json(),
            json!({"kind": "RateLimited", "message": OWN_KEY_RATE_LIMITED})
        );
        assert_eq!(
            AiError::ModelUnknown { model: "m".into() }.to_json()["model"],
            "m"
        );
        let unrecorded = AiError::Unrecorded {
            hash: "h".into(),
            path: "/p".into(),
        }
        .to_json();
        assert_eq!(
            (unrecorded["hash"].as_str(), unrecorded["path"].as_str()),
            (Some("h"), Some("/p"))
        );
    }

    #[test]
    fn kind_names_every_variant() {
        assert_eq!(AiError::InvalidAnswer.kind(), "InvalidAnswer");
        assert_eq!(AiError::LocalNotLocal.kind(), "LocalNotLocal");
        assert_eq!(AiError::ProviderError { code: 500 }.kind(), "ProviderError");
    }
}
