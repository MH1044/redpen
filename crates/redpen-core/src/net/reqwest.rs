//! The production [`HttpTransport`]: reqwest with rustls, certificates checked against the
//! operating system's store (`rustls-platform-verifier`). Connect timeout 5 s; each request
//! carries its own overall timeout (docs/PUBLIK-INTEGRATION.md §6).
//!
//! - Redirects are never followed, so a request goes only to the host its URL names: a local
//!   server cannot send a call elsewhere (invariant 12) and a key is never replayed to another
//!   host.
//! - Requests to this computer (`localhost`, `127.0.0.1`, `::1`) never go through a proxy, for
//!   the same reason; other requests use the system's proxy settings.
//! - A key travels only as a header value marked sensitive. Errors carry no URL, no header and
//!   no body: reqwest's own messages name the URL, so they are reduced to a kind.

use std::time::Duration;

use super::{Body, HttpRequest, HttpResponse, HttpTransport, Method, TransportError, url_host};

/// The connect timeout of every request.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// HTTP over the network, for the app and the command line's live providers. Never used in
/// tests.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    remote: ::reqwest::Client,
    loopback: ::reqwest::Client,
}

impl ReqwestTransport {
    /// The transport, or [`TransportError::Tls`] if the TLS set-up (the system certificate
    /// store) cannot be loaded.
    pub fn new() -> Result<Self, TransportError> {
        let unavailable = |_| TransportError::Tls("the TLS set-up could not be loaded".into());
        Ok(Self {
            remote: builder().build().map_err(unavailable)?,
            loopback: builder().no_proxy().build().map_err(unavailable)?,
        })
    }
}

fn builder() -> ::reqwest::ClientBuilder {
    ::reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(::reqwest::redirect::Policy::none())
        .user_agent(concat!("redpen/", env!("CARGO_PKG_VERSION")))
}

/// Whether `url` names this computer, so it must bypass any proxy.
fn is_loopback(url: &str) -> bool {
    url_host(url).is_some_and(|h| matches!(h.as_str(), "localhost" | "127.0.0.1" | "::1"))
}

#[async_trait::async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let client = if is_loopback(&req.url) {
            &self.loopback
        } else {
            &self.remote
        };
        let method = match req.method {
            Method::Get => ::reqwest::Method::GET,
            Method::Post => ::reqwest::Method::POST,
        };
        let mut builder = client.request(method, &req.url).timeout(req.timeout);
        let mut has_content_type = false;
        for (name, value) in &req.headers {
            has_content_type |= name.eq_ignore_ascii_case("content-type");
            // A value that cannot be a header (a key pasted with a line break) cannot be sent.
            let mut header = ::reqwest::header::HeaderValue::from_str(value.expose())
                .map_err(|_| TransportError::Offline)?;
            header.set_sensitive(value.is_secret());
            builder = builder.header(name.as_str(), header);
        }
        if let Body::Json(value) = &req.body {
            if !has_content_type {
                builder = builder.header("content-type", "application/json");
            }
            builder = builder.body(serde_json::to_vec(value).expect("a JSON value serialises"));
        }
        let resp = builder.send().await.map_err(classify)?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let body = resp.bytes().await.map_err(classify)?.to_vec();
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// The kind of a reqwest failure, without its message (which names the URL).
fn classify(e: ::reqwest::Error) -> TransportError {
    if e.is_connect() {
        if mentions_tls(&e) {
            return TransportError::Tls("the secure connection could not be set up".into());
        }
        return TransportError::Offline;
    }
    if e.is_timeout() {
        return TransportError::Timeout;
    }
    TransportError::Offline
}

/// Whether a connection failure came from the TLS handshake or the certificate check.
fn mentions_tls(e: &::reqwest::Error) -> bool {
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        let text = s.to_string().to_ascii_lowercase();
        if text.contains("certificate") || text.contains("tls") || text.contains("handshake") {
            return true;
        }
        source = s.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_with_the_system_certificate_store() {
        assert!(ReqwestTransport::new().is_ok());
    }

    #[test]
    fn loopback_addresses_skip_the_proxy() {
        assert!(is_loopback("http://localhost:11434/api/tags"));
        assert!(is_loopback("http://127.0.0.1:8080/v1/chat/completions"));
        assert!(is_loopback("http://[::1]:11434/v1/models"));
        assert!(!is_loopback("https://api.example.com/v1/models"));
        assert!(!is_loopback("http://localhost@example.com/"));
    }
}
