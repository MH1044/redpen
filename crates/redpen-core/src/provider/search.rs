//! Web search results and errors (docs/PUBLIK-CONTRACT.md §6a, docs/PUBLIK-INTEGRATION.md §6.4).
//!
//! The app's `publik::search` sends `POST {base_url}/search` with `{"q", "count"}` and the key
//! in the `Authorization` header only; the answer is read here, so the plagiarism check (module
//! 09) can fake [`SearchOutcome`] and [`SearchError`] without the app crate. A search is never
//! retried automatically, carries no idempotency key, and a 502 or 504 costs nothing.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::errors::AiError;
use super::publik_errors::{DISCONNECTED, PUBLIK_BUSY, map_publik_error};
use crate::net::{TransportError, url_host};

/// The search endpoint under publik API's base URL.
pub const SEARCH_PATH: &str = "/search";
/// How long a search may take (docs/PUBLIK-INTEGRATION.md §6.4; connect 5 s).
pub const SEARCH_TIMEOUT_SECONDS: u64 = 30;

/// One web result: what the plagiarism check compares and cites. Result pages are never
/// requested; `url` is only ever handed to the system browser when the user opens it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub site: String,
    /// The page's date as the search provider gives it (`2026-09-30`), when it gives one.
    pub age: Option<String>,
}

/// The answer to one search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOutcome {
    pub results: Vec<SearchResult>,
    /// The body's `charge_micros`, else the `x-publik-charge-micros` header; `None` off publik
    /// API and from the cache.
    pub charge_micros: Option<u64>,
    /// Whether the answer came from the cache, so nothing was sent.
    pub from_cache: bool,
}

/// Why a search gave no results. `Display` is the message the interface shows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    /// 402: publik's message and its one link; the check stops.
    #[error("{message}")]
    NeedsCredit {
        message: String,
        top_up_url: Option<String>,
    },
    /// 502 or 504: the search provider failed and nothing was charged; only that sentence is
    /// marked, and Try again is offered.
    #[error("The search provider failed for this sentence; nothing was charged.")]
    ProviderFailed,
    /// 401, or 403 `install_revoked`: offer Reconnect.
    #[error("{}", DISCONNECTED)]
    Disconnected,
    /// 429: publik's message.
    #[error("{0}")]
    RateLimited(String),
    /// 503.
    #[error("{}", PUBLIK_BUSY)]
    Busy,
    /// No answer within 30 seconds.
    #[error("publik API did not answer within 30 seconds. Try again.")]
    Timeout,
    /// No connection could be made.
    #[error("No connection. A plagiarism check needs a connection; checking still works.")]
    Offline,
    /// Any other status, or a request the fake transport has no entry for (code 0).
    #[error("publik API {code}: {message}")]
    Other { code: u16, message: String },
    /// A 2xx whose body is not a search answer.
    #[error("The search answer could not be used.")]
    InvalidAnswer,
}

impl SearchError {
    /// The variant's name, as the interface receives it.
    pub fn kind(&self) -> &'static str {
        match self {
            SearchError::NeedsCredit { .. } => "NeedsCredit",
            SearchError::ProviderFailed => "ProviderFailed",
            SearchError::Disconnected => "Disconnected",
            SearchError::RateLimited(_) => "RateLimited",
            SearchError::Busy => "Busy",
            SearchError::Timeout => "Timeout",
            SearchError::Offline => "Offline",
            SearchError::Other { .. } => "Other",
            SearchError::InvalidAnswer => "InvalidAnswer",
        }
    }

    /// `{"kind", "message"}` plus `top_up_url` or `code` where the variant has one.
    pub fn to_json(&self) -> Value {
        let mut v = json!({"kind": self.kind(), "message": self.to_string()});
        match self {
            SearchError::NeedsCredit { top_up_url, .. } => v["top_up_url"] = json!(top_up_url),
            SearchError::Other { code, .. } => v["code"] = json!(code),
            _ => {}
        }
        v
    }
}

impl Serialize for SearchError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

/// The body of a search: exactly `{"q", "count"}`.
pub fn search_body(q: &str, count: u8) -> Value {
    json!({"q": q, "count": count})
}

/// Reads a 2xx search answer, `{"results": [{"url", "title", "snippet", "site", "age"}],
/// "charge_micros"}`. A result without a `url` is skipped; a missing `site` is the URL's host.
/// `header_charge` is the response's `x-publik-charge-micros`, used when the body has none.
pub fn parse_search_answer(
    body: &[u8],
    header_charge: Option<&str>,
) -> Result<SearchOutcome, SearchError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| SearchError::InvalidAnswer)?;
    let items = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or(SearchError::InvalidAnswer)?;
    let text = |item: &Value, name: &str| {
        item.get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default()
    };
    let results = items
        .iter()
        .filter_map(|item| {
            let url = text(item, "url");
            if url.trim().is_empty() {
                return None;
            }
            let mut site = text(item, "site");
            if site.is_empty() {
                site = url_host(&url).unwrap_or_default();
            }
            Some(SearchResult {
                title: text(item, "title"),
                snippet: text(item, "snippet"),
                site,
                age: item
                    .get("age")
                    .and_then(Value::as_str)
                    .filter(|a| !a.trim().is_empty())
                    .map(str::to_string),
                url,
            })
        })
        .collect();
    let charge_micros = value
        .get("charge_micros")
        .and_then(Value::as_u64)
        .or_else(|| header_charge.and_then(|h| h.trim().parse().ok()));
    Ok(SearchOutcome {
        results,
        charge_micros,
        from_cache: false,
    })
}

/// The error for a search response that is not a 2xx: 402 → `NeedsCredit`; 502 and 504 →
/// `ProviderFailed`; the rest as for chat.
pub fn search_error(status: u16, body: &[u8]) -> SearchError {
    match status {
        502 | 504 => SearchError::ProviderFailed,
        _ => match map_publik_error(status, body) {
            AiError::NeedsCredit {
                message,
                top_up_url,
            } => SearchError::NeedsCredit {
                message,
                top_up_url,
            },
            AiError::Disconnected => SearchError::Disconnected,
            AiError::PublikRateLimited { message } => SearchError::RateLimited(message),
            AiError::PublikBusy => SearchError::Busy,
            AiError::PublikHttp { code, message } => SearchError::Other { code, message },
            other => SearchError::Other {
                code: status,
                message: other.to_string(),
            },
        },
    }
}

/// The error for a search that got no response.
pub fn search_transport_error(e: TransportError) -> SearchError {
    match e {
        TransportError::Timeout => SearchError::Timeout,
        TransportError::Offline | TransportError::Tls(_) => SearchError::Offline,
        unrecorded @ TransportError::Unrecorded { .. } => SearchError::Other {
            code: 0,
            message: unrecorded.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(v: Value) -> Vec<u8> {
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn the_body_is_q_and_count_only() {
        assert_eq!(
            crate::net::canonical_json(&search_body("\"largest coral reef system\"", 5)),
            r#"{"count":5,"q":"\"largest coral reef system\""}"#
        );
    }

    #[test]
    fn an_answer_gives_its_results_and_charge() {
        let body = bytes(json!({
            "object": "search.results", "query": "q", "count": 2, "more": true,
            "results": [
                {"url": "https://en.example.org/wiki/Reef", "title": "Reef", "snippet": "The largest…",
                 "site": "en.example.org", "age": "2026-09-30"},
                {"url": "https://www.example.com/reefs", "title": "Reefs", "snippet": "…", "site": ""},
                {"title": "no url"}
            ],
            "charge_micros": 10000, "request_id": "req_1"
        }));
        let outcome = parse_search_answer(&body, None).unwrap();
        assert_eq!(outcome.results.len(), 2);
        assert_eq!(outcome.results[0].age.as_deref(), Some("2026-09-30"));
        assert_eq!(outcome.results[1].site, "www.example.com");
        assert_eq!(outcome.results[1].age, None);
        assert_eq!(outcome.charge_micros, Some(10000));
        assert!(!outcome.from_cache);
        let without = bytes(json!({"results": []}));
        assert_eq!(
            parse_search_answer(&without, Some("10000"))
                .unwrap()
                .charge_micros,
            Some(10000)
        );
        assert_eq!(
            parse_search_answer(&without, None).unwrap().charge_micros,
            None
        );
    }

    #[test]
    fn a_body_without_results_is_invalid() {
        assert_eq!(
            parse_search_answer(b"{}", None),
            Err(SearchError::InvalidAnswer)
        );
        assert_eq!(
            parse_search_answer(b"<html>", None),
            Err(SearchError::InvalidAnswer)
        );
    }

    #[test]
    fn errors_map_as_the_contract_says() {
        let credit = bytes(json!({"error": {"message": "Add a plan or pack.",
                                            "top_up_url": "https://publikhq.com/wallet"}}));
        assert_eq!(
            search_error(402, &credit),
            SearchError::NeedsCredit {
                message: "Add a plan or pack.".into(),
                top_up_url: Some("https://publikhq.com/wallet".into())
            }
        );
        for code in [502, 504] {
            let e = search_error(code, b"{}");
            assert_eq!(e, SearchError::ProviderFailed);
            assert_eq!(
                e.to_string(),
                "The search provider failed for this sentence; nothing was charged."
            );
        }
        assert_eq!(search_error(401, b"{}"), SearchError::Disconnected);
        assert_eq!(search_error(503, b"{}"), SearchError::Busy);
        assert_eq!(
            search_error(429, &bytes(json!({"error": {"message": "Slow down."}}))),
            SearchError::RateLimited("Slow down.".into())
        );
        let bad = search_error(400, &bytes(json!({"error": {"message": "q is too long."}})));
        assert_eq!(bad.to_string(), "publik API 400: q is too long.");
        assert_eq!(bad.to_json()["code"], 400);
    }

    #[test]
    fn transport_failures_have_their_messages() {
        assert_eq!(
            search_transport_error(TransportError::Timeout).to_string(),
            "publik API did not answer within 30 seconds. Try again."
        );
        assert_eq!(
            search_transport_error(TransportError::Offline).to_string(),
            "No connection. A plagiarism check needs a connection; checking still works."
        );
        assert!(matches!(
            search_transport_error(TransportError::Unrecorded {
                hash: "h".into(),
                path: "/search".into()
            }),
            SearchError::Other { code: 0, .. }
        ));
    }

    #[test]
    fn errors_serialise_with_kind_and_message() {
        let e = SearchError::NeedsCredit {
            message: "m".into(),
            top_up_url: None,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"kind": "NeedsCredit", "message": "m", "top_up_url": null})
        );
        assert_eq!(SearchError::Busy.kind(), "Busy");
        assert_eq!(
            SearchError::InvalidAnswer.to_string(),
            "The search answer could not be used."
        );
        assert_eq!(SearchError::Disconnected.kind(), "Disconnected");
    }
}
