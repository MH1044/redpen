//! HTTP, behind a trait (docs/ARCHITECTURE.md §3 and §7, invariants 10 and 11). The production
//! transport (reqwest with rustls, module 02) and the test transport ([`FakeTransport`]) both
//! implement [`HttpTransport`]; the core never opens a socket itself.
//!
//! A request is identified by [`request_hash`]: the method, the path after the provider's base
//! path and the canonical JSON body. The fake answers by that hash from a recording and fails on
//! any request it has no entry for. Headers never count, so a key never influences a recording.

pub mod fake;
pub mod hash;
pub mod recording;

use std::fmt;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

pub use fake::{FakeEntry, FakeError, FakeTransport};
pub use hash::{canonical_json, request_hash, request_path};
pub use recording::{Recorder, Recording, RecordingEntry};

/// The HTTP methods Redpen uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A header value. A key is wrapped as [`HeaderValue::Secret`], whose `Debug` output is `***`,
/// so a request logged or printed while debugging never shows it (invariant 6).
#[derive(Clone)]
pub enum HeaderValue {
    Plain(String),
    Secret(SecretString),
}

impl HeaderValue {
    /// A plain header value.
    pub fn plain(value: impl Into<String>) -> Self {
        HeaderValue::Plain(value.into())
    }

    /// A header value that must never be printed.
    pub fn secret(value: impl Into<String>) -> Self {
        HeaderValue::Secret(SecretString::from(value.into()))
    }

    /// The value to send. Only the transport calls this.
    pub fn expose(&self) -> &str {
        match self {
            HeaderValue::Plain(s) => s,
            HeaderValue::Secret(s) => s.expose_secret(),
        }
    }

    /// Whether the value is a secret.
    pub fn is_secret(&self) -> bool {
        matches!(self, HeaderValue::Secret(_))
    }
}

impl fmt::Debug for HeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderValue::Plain(s) => f.debug_tuple("Plain").field(s).finish(),
            HeaderValue::Secret(_) => f.write_str("***"),
        }
    }
}

impl PartialEq for HeaderValue {
    fn eq(&self, other: &Self) -> bool {
        self.is_secret() == other.is_secret() && self.expose() == other.expose()
    }
}

/// A request body.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Empty,
    Json(serde_json::Value),
}

/// One outbound request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, HeaderValue)>,
    pub body: Body,
    pub timeout: Duration,
}

impl HttpRequest {
    /// A JSON `POST` with a 30 second timeout and no headers.
    pub fn post_json(url: impl Into<String>, body: serde_json::Value) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: Vec::new(),
            body: Body::Json(body),
            timeout: Duration::from_secs(30),
        }
    }

    /// A `GET` with a 30 second timeout and no headers.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: Body::Empty,
            timeout: Duration::from_secs(30),
        }
    }

    /// Adds a header.
    pub fn header(mut self, name: impl Into<String>, value: HeaderValue) -> Self {
        self.headers.push((name.into(), value));
        self
    }

    /// Sets the timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// One response. Header names are lower-cased by every transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// The first header with this name, compared case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The body parsed as JSON, if it is JSON.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// What can go wrong below the HTTP status code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// No answer within the request's timeout.
    #[error("the request timed out")]
    Timeout,
    /// No connection could be made.
    #[error("no connection")]
    Offline,
    /// The TLS handshake failed; the message never contains request data.
    #[error("TLS failed: {0}")]
    Tls(String),
    /// The fake transport had no entry for this request (docs/TESTING.md §1).
    #[error(
        "Unrecorded request {hash} to {path}; body written to target/unrecorded/{hash}.json. Add a hand-written entry, or ask the maintainer to record it."
    )]
    Unrecorded { hash: String, path: String },
}

/// Sends requests. Production uses reqwest; tests use [`FakeTransport`].
#[async_trait::async_trait]
pub trait HttpTransport: Send + Sync {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_header_value_debug_prints_stars() {
        let value = HeaderValue::secret("Bearer sk-test-a");
        assert_eq!(format!("{value:?}"), "***");
        let plain = HeaderValue::plain("application/json");
        assert_eq!(format!("{plain:?}"), "Plain(\"application/json\")");
    }

    #[test]
    fn request_debug_never_shows_a_secret_header() {
        let req = HttpRequest::get("https://publikhq.com/api/v1/wallet")
            .header("authorization", HeaderValue::secret("Bearer pk_live_a"));
        let debug = format!("{req:?}");
        assert!(!debug.contains("pk_live_a"));
        assert!(debug.contains("***"));
    }

    #[test]
    fn response_header_lookup_ignores_case() {
        let resp = HttpResponse {
            status: 200,
            headers: vec![("x-publik-charge-micros".into(), "210".into())],
            body: b"{}".to_vec(),
        };
        assert_eq!(resp.header("X-Publik-Charge-Micros"), Some("210"));
        assert_eq!(resp.header("x-publik-balance"), None);
        assert_eq!(resp.json(), Some(serde_json::json!({})));
    }

    #[test]
    fn unrecorded_message_names_the_hash_and_the_file() {
        let err = TransportError::Unrecorded {
            hash: "abc".into(),
            path: "/chat/completions".into(),
        };
        assert_eq!(
            err.to_string(),
            "Unrecorded request abc to /chat/completions; body written to target/unrecorded/abc.json. Add a hand-written entry, or ask the maintainer to record it."
        );
    }
}
