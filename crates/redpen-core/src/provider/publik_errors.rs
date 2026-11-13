//! publik API's errors and the one message and way out each gets (docs/PUBLIK-CONTRACT.md §8,
//! docs/PUBLIK-INTEGRATION.md §9, docs/modules/03-publik.md, Interfaces).
//!
//! publik API answers an error with a JSON body, `{"error": {"message", "code", ...}}`, and a
//! 402 adds `error.top_up_url`, the one link the interface may show. The messages publik writes
//! are shown as they are (trimmed, at most 500 characters, control characters dropped); when a
//! body has none, the fixed texts below stand in. [`map_publik_error`] is the one mapping, used
//! by [`super::AiClient`] for chat and by the app's `publik.rs` for `/wallet` and web search.

use serde_json::Value;

use super::errors::AiError;
use crate::net::url_host;

/// The message of [`AiError::NeedsConsent`]: the consent view's title, shown instead.
pub const NEEDS_CONSENT: &str = "Choose how Redpen uses AI.";
/// 401, or 403 `install_revoked`.
pub const DISCONNECTED: &str = "publik API disconnected this computer.";
/// 503.
pub const PUBLIK_BUSY: &str = "publik API is busy; try again in a moment";
/// A build without a `pat_` app token.
pub const NO_APP_TOKEN: &str = "This build has no publik API app token. Use your own key for now.";
/// `POST /installs` got no answer at all.
pub const UNREACHABLE: &str = "publik API is unreachable right now. Use your own key for now.";
/// The way out the interface puts after a 402's message and link.
pub const NEEDS_CREDIT_WAY_OUT: &str = "or use your own key or a local model in Settings";
/// A 402 whose body carries no message.
pub const NEEDS_CREDIT_FALLBACK: &str = "Your publik balance does not cover this request.";
/// A 429 whose body carries no message.
pub const RATE_LIMITED_FALLBACK: &str =
    "publik API says this computer is sending too many requests. Wait a moment and try again.";
/// A 429 on `POST /installs` whose body carries no message.
pub const PROVISION_BUSY_FALLBACK: &str =
    "publik API is busy setting up computers right now. Use your own key for now.";
/// Any other error whose body carries no message: "publik API 500: The request failed."
pub const HTTP_FALLBACK: &str = "The request failed.";

/// The error code publik sends when this computer's install was revoked.
pub const INSTALL_REVOKED: &str = "install_revoked";

/// The longest message from publik that is shown.
const MAX_MESSAGE_CHARS: usize = 500;

/// "publik API could not set up this computer (500). Use your own key for now."
pub fn provision_failed_message(code: u16) -> String {
    format!("publik API could not set up this computer ({code}). Use your own key for now.")
}

/// What an error body says, each part only when present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublikErrorBody {
    /// `error.message`, cleaned for display.
    pub message: Option<String>,
    /// `error.code` (or `error.type`), such as `install_revoked` or `invalid_api_key`.
    pub code: Option<String>,
    /// `error.top_up_url`, only when it is an `https` address with a host.
    pub top_up_url: Option<String>,
}

/// Reads an error body. Anything that is not the documented shape gives empty parts, never a
/// failure: the status code alone still decides the error.
pub fn parse_error_body(body: &[u8]) -> PublikErrorBody {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return PublikErrorBody::default();
    };
    let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    match value.get("error") {
        Some(Value::Object(error)) => PublikErrorBody {
            message: text(error.get("message")).and_then(|m| clean_message(&m)),
            code: text(error.get("code")).or_else(|| text(error.get("type"))),
            top_up_url: text(error.get("top_up_url")).filter(|u| is_https_link(u)),
        },
        // The device-flow shape, `{"error": "code", "message": "..."}`.
        Some(Value::String(code)) => PublikErrorBody {
            message: text(value.get("message")).and_then(|m| clean_message(&m)),
            code: Some(code.clone()),
            top_up_url: None,
        },
        _ => PublikErrorBody::default(),
    }
}

/// The one link of a 402: `error.top_up_url`, when it is an `https` address.
pub fn top_up_url_from_402(body: &[u8]) -> Option<String> {
    parse_error_body(body).top_up_url
}

/// The [`AiError`] for a publik API response that is not a 2xx (contract §8):
///
/// | Status | Error |
/// |---|---|
/// | 402 | `NeedsCredit {message, top_up_url}` |
/// | 401, 403 `install_revoked` | `Disconnected` |
/// | 429 | `PublikRateLimited {message}` |
/// | 503 | `PublikBusy` |
/// | any other | `PublikHttp {code, message}` |
pub fn map_publik_error(status: u16, body: &[u8]) -> AiError {
    let parsed = parse_error_body(body);
    match status {
        402 => AiError::NeedsCredit {
            message: parsed
                .message
                .unwrap_or_else(|| NEEDS_CREDIT_FALLBACK.to_string()),
            top_up_url: parsed.top_up_url,
        },
        401 => AiError::Disconnected,
        403 if parsed.code.as_deref() == Some(INSTALL_REVOKED) => AiError::Disconnected,
        429 => AiError::PublikRateLimited {
            message: parsed
                .message
                .unwrap_or_else(|| RATE_LIMITED_FALLBACK.to_string()),
        },
        503 => AiError::PublikBusy,
        code => AiError::PublikHttp {
            code,
            message: parsed.message.unwrap_or_else(|| HTTP_FALLBACK.to_string()),
        },
    }
}

/// Whether `url` is an `https` address with a host: the only kind of link Redpen hands to the
/// system browser.
pub fn is_https_link(url: &str) -> bool {
    let url = url.trim();
    url.len() > "https://".len()
        && url[..8].eq_ignore_ascii_case("https://")
        && url_host(url).is_some()
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// A message as shown: each control character (a line break, an escape) becomes a space, then
/// trimmed, at most 500 characters; `None` when nothing is left.
fn clean_message(message: &str) -> Option<String> {
    let cleaned: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_MESSAGE_CHARS)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(v: Value) -> Vec<u8> {
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn a_402_gives_its_message_and_one_link() {
        for (message, url) in [
            (
                "Link this computer for $0.05 of free use.",
                "https://publikhq.com/claim/abc",
            ),
            (
                "Your publik balance is $0.00. Add a plan or pack.",
                "https://publikhq.com/wallet/add",
            ),
        ] {
            let b = body(
                json!({"error": {"type": "insufficient_balance", "message": message,
                                          "top_up_url": url}}),
            );
            assert_eq!(
                map_publik_error(402, &b),
                AiError::NeedsCredit {
                    message: message.into(),
                    top_up_url: Some(url.into())
                }
            );
            assert_eq!(top_up_url_from_402(&b).as_deref(), Some(url));
        }
        assert_eq!(
            map_publik_error(402, b"not json"),
            AiError::NeedsCredit {
                message: NEEDS_CREDIT_FALLBACK.into(),
                top_up_url: None
            }
        );
    }

    #[test]
    fn a_link_must_be_https() {
        for bad in [
            "http://publikhq.com/claim",
            "javascript:alert(1)",
            "https://",
            "https://publikhq.com/a b",
            "file:///c/x",
        ] {
            let b = body(json!({"error": {"message": "m", "top_up_url": bad}}));
            assert_eq!(top_up_url_from_402(&b), None, "{bad}");
        }
        assert!(is_https_link("https://publikhq.com/wallet"));
        assert!(is_https_link("HTTPS://publikhq.com/wallet"));
    }

    #[test]
    fn disconnection_is_401_and_403_install_revoked() {
        for b in [
            json!({"error": {"code": "invalid_api_key", "message": "Invalid key"}}),
            json!({"error": {"code": "key_revoked", "message": "Revoked"}}),
            json!({}),
        ] {
            assert_eq!(map_publik_error(401, &body(b)), AiError::Disconnected);
        }
        let revoked = body(json!({"error": {"code": "install_revoked", "message": "Gone"}}));
        assert_eq!(map_publik_error(403, &revoked), AiError::Disconnected);
        let as_type = body(json!({"error": {"type": "install_revoked"}}));
        assert_eq!(map_publik_error(403, &as_type), AiError::Disconnected);
        let other = body(json!({"error": {"code": "forbidden", "message": "Not allowed."}}));
        assert_eq!(
            map_publik_error(403, &other),
            AiError::PublikHttp {
                code: 403,
                message: "Not allowed.".into()
            }
        );
    }

    #[test]
    fn rate_limits_busy_and_other_codes() {
        let slow = body(json!({"error": {"message": "Slow down."}}));
        assert_eq!(
            map_publik_error(429, &slow),
            AiError::PublikRateLimited {
                message: "Slow down.".into()
            }
        );
        assert_eq!(
            map_publik_error(429, b""),
            AiError::PublikRateLimited {
                message: RATE_LIMITED_FALLBACK.into()
            }
        );
        assert_eq!(map_publik_error(503, &slow), AiError::PublikBusy);
        let model = body(json!({"error": {"message": "The model returned an error."}}));
        assert_eq!(
            map_publik_error(500, &model).to_string(),
            "publik API 500: The model returned an error."
        );
        assert_eq!(
            map_publik_error(400, b"{}").to_string(),
            "publik API 400: The request failed."
        );
    }

    #[test]
    fn messages_are_cleaned_and_capped() {
        let long = "x".repeat(800);
        let b = body(json!({"error": {"message": format!("  a\u{1b}[31m{long}\n ")}}));
        let parsed = parse_error_body(&b);
        let message = parsed.message.unwrap();
        assert!(message.starts_with("a [31m"), "{message}");
        assert!(!message.contains('\u{1b}'));
        assert_eq!(message.chars().count(), 500);
        let blank = body(json!({"error": {"message": " \n "}}));
        assert_eq!(parse_error_body(&blank).message, None);
    }

    #[test]
    fn the_device_flow_shape_is_read_too() {
        let b = body(json!({"error": "install_revoked", "message": "This install was revoked."}));
        let parsed = parse_error_body(&b);
        assert_eq!(parsed.code.as_deref(), Some("install_revoked"));
        assert_eq!(parsed.message.as_deref(), Some("This install was revoked."));
        assert_eq!(map_publik_error(403, &b), AiError::Disconnected);
    }

    #[test]
    fn the_provision_message_names_the_code() {
        assert_eq!(
            provision_failed_message(500),
            "publik API could not set up this computer (500). Use your own key for now."
        );
    }
}
