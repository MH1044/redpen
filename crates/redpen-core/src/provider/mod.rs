//! One OpenAI-compatible chat client for every provider (docs/modules/02-provider-layer.md):
//! the user's own key, a local model, the fake provider, and publik API (module 03).
//!
//! [`AiClient::chat`] sends `POST {base_url}/chat/completions` with `{model, messages,
//! max_tokens, temperature}`, not streamed, and returns `choices[0].message.content`. Around it:
//! the answer cache ([`cache`]), retries for the own key and local only ([`retry`]), the
//! localhost rule and Ollama's model choice ([`local`]), daily usage counts and the opt-in sent
//! log in `redpen.db`, and errors a user can act on ([`AiError`]).
//!
//! For publik API (module 03): the tier rule ([`tier`]), the idempotency key every chat call
//! carries ([`idempotency_key`]), the mapping of its errors ([`publik_errors`]), the
//! [`ProviderResolver`] the app implements over Settings and the credential ([`resolver`]), and
//! the web search answer ([`search`]).

pub mod cache;
pub mod client;
pub mod errors;
pub mod local;
pub mod publik_errors;
pub mod resolved;
pub mod resolver;
pub mod retry;
pub mod search;
pub mod tier;

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use cache::{cache_key, idempotency_key};
pub use client::{AiClient, PrivacyOptions, ProviderTest};
pub use errors::{AiError, OWN_KEY_RATE_LIMITED};
pub use local::{LocalModel, pick_local_model};
pub use publik_errors::{map_publik_error, top_up_url_from_402};
pub use resolved::{ProviderKind, ResolvedProvider};
pub use resolver::{FixedResolver, ProviderResolver};
pub use retry::RETRY_DELAYS;
pub use search::{SearchError, SearchOutcome, SearchResult};
pub use secrecy::{ExposeSecret, SecretString};
pub use tier::{ClaimState, RewriteQuality, TierContext, publik_tier};

/// What a call is for. `usage_daily.feature` and `sent_log.feature` hold the kebab-case name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Feature {
    FixGrammar,
    Clearer,
    Shorter,
    MoreFormal,
    Friendlier,
    Custom,
    /// A provider's Test button, and the command line's `chat`.
    Test,
}

impl Feature {
    pub fn as_str(&self) -> &'static str {
        match self {
            Feature::FixGrammar => "fix-grammar",
            Feature::Clearer => "clearer",
            Feature::Shorter => "shorter",
            Feature::MoreFormal => "more-formal",
            Feature::Friendlier => "friendlier",
            Feature::Custom => "custom",
            Feature::Test => "test",
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a call's answer may be kept (see [`cache`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    Disk,
    Memory,
    Off,
}

/// One chat message, `{"role", "content"}` on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new("system", content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", content)
    }
}

/// One chat call: what to send, how long to wait, and how to cache the answer.
#[derive(Debug, Clone)]
pub struct ChatCall {
    pub feature: Feature,
    pub prompt_id: &'static str,
    pub prompt_version: u32,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub temperature: f32,
    /// 30 s on the own key and Publik, 120 s on a local model ([`chat_timeout`]).
    pub timeout: Duration,
    /// The input fields of the cache key (docs/PUBLIK-INTEGRATION.md §5).
    pub cache_input: serde_json::Value,
    pub cache: CachePolicy,
    /// The n-th Try again after an invalid answer (0 for a first call). On publik API it sets
    /// the idempotency key ([`idempotency_key`]); above 0 the call is always sent, never
    /// answered from the cache, since the cached answer is the one that failed. A valid answer
    /// is still stored under the cache key K, which never includes it.
    pub validation_retry: u32,
}

/// A chat answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    /// `choices[0].message.content`.
    pub content: String,
    /// Whether the answer came from the cache, so nothing was sent.
    pub from_cache: bool,
    /// What the status sink read from the response (publik API's `x-publik-charge-micros`);
    /// `None` from the cache and off Publik.
    pub charge_micros: Option<u64>,
    /// The response's `x-publik-request-id` (or `x-request-id`), when it has one.
    pub request_id: Option<String>,
}

/// Reads the headers of every chat response. Module 03 implements it for publik API's status
/// headers (`PublikStatusSink` in the app); the default reads nothing.
pub trait StatusSink: Send + Sync {
    /// Called once per response (each attempt, any status). Returns the call's charge in
    /// micro-dollars, which `usage_daily.charge_micros` adds up.
    fn on_response(
        &self,
        kind: ProviderKind,
        feature: Feature,
        status: u16,
        headers: &[(String, String)],
    ) -> Option<u64> {
        let _ = (kind, feature, status, headers);
        None
    }

    /// Called once when a chat call fails, with the error the caller receives, so the publik
    /// status can keep the one link of a 402. The default ignores it.
    fn on_error(&self, kind: ProviderKind, error: &AiError) {
        let _ = (kind, error);
    }
}

/// The status sink that reads nothing: own key, local and the fake provider.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopStatusSink;

impl StatusSink for NoopStatusSink {}

impl StatusSink for () {}

/// The connect timeout of every request (docs/PUBLIK-INTEGRATION.md §6).
pub const CONNECT_TIMEOUT: Duration = crate::net::reqwest::CONNECT_TIMEOUT;

/// A chat call's timeout for this kind of provider: 120 s on a local model, 30 s elsewhere.
pub fn chat_timeout(kind: ProviderKind) -> Duration {
    match kind {
        ProviderKind::Local => Duration::from_secs(120),
        ProviderKind::Publik | ProviderKind::OwnKey | ProviderKind::Fake => Duration::from_secs(30),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_have_their_database_names() {
        let all = [
            Feature::FixGrammar,
            Feature::Clearer,
            Feature::Shorter,
            Feature::MoreFormal,
            Feature::Friendlier,
            Feature::Custom,
            Feature::Test,
        ];
        for f in all {
            assert_eq!(
                serde_json::to_value(f).unwrap(),
                serde_json::json!(f.as_str())
            );
        }
        assert_eq!(Feature::MoreFormal.as_str(), "more-formal");
        assert_eq!(Feature::FixGrammar.to_string(), "fix-grammar");
    }

    #[test]
    fn messages_serialise_as_role_and_content() {
        assert_eq!(
            serde_json::to_value(Message::user("hi")).unwrap(),
            serde_json::json!({"role": "user", "content": "hi"})
        );
        assert_eq!(Message::system("s").role, "system");
        assert_eq!(Message::assistant("a").role, "assistant");
    }

    #[test]
    fn the_default_sink_reads_nothing() {
        let headers = vec![("x-publik-charge-micros".to_string(), "210".to_string())];
        assert_eq!(
            NoopStatusSink.on_response(ProviderKind::Publik, Feature::Clearer, 200, &headers),
            None
        );
        assert_eq!(
            ().on_response(ProviderKind::OwnKey, Feature::Clearer, 200, &headers),
            None
        );
    }

    #[test]
    fn timeouts_are_30_seconds_and_120_on_local() {
        assert_eq!(chat_timeout(ProviderKind::OwnKey), Duration::from_secs(30));
        assert_eq!(chat_timeout(ProviderKind::Publik), Duration::from_secs(30));
        assert_eq!(chat_timeout(ProviderKind::Local), Duration::from_secs(120));
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(5));
    }
}
