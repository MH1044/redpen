//! Automatic retries (docs/PUBLIK-INTEGRATION.md §6): own-key and local calls try twice more,
//! after 0.5 s and then 1 s, on a timeout, a 429 or a 5xx, and never on any other 4xx. The rule
//! is keyed on the provider kind alone, so a publik API call can never be retried (invariant 9):
//! Publik settles usage even when the client gives up, and a retry could charge twice.

use std::time::Duration;

use super::resolved::ProviderKind;
use crate::net::{HttpResponse, TransportError};

/// The waits before the second and the third attempt.
pub const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1000)];

/// The waits before each further attempt for this kind of provider: two for the own key and a
/// local server, none for publik API and the fake provider.
pub fn retry_delays(kind: ProviderKind) -> &'static [Duration] {
    match kind {
        ProviderKind::OwnKey | ProviderKind::Local => &RETRY_DELAYS,
        ProviderKind::Publik | ProviderKind::Fake => &[],
    }
}

/// Whether an attempt's outcome is worth another attempt: a timeout, a 429 or a 5xx.
pub fn is_retryable(outcome: &Result<HttpResponse, TransportError>) -> bool {
    match outcome {
        Err(TransportError::Timeout) => true,
        Err(_) => false,
        Ok(resp) => resp.status == 429 || (500..=599).contains(&resp.status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: u16) -> Result<HttpResponse, TransportError> {
        Ok(HttpResponse {
            status: code,
            headers: Vec::new(),
            body: Vec::new(),
        })
    }

    #[test]
    fn publik_never_retries() {
        assert!(retry_delays(ProviderKind::Publik).is_empty());
        assert!(retry_delays(ProviderKind::Fake).is_empty());
        assert_eq!(
            retry_delays(ProviderKind::OwnKey),
            [Duration::from_millis(500), Duration::from_millis(1000)]
        );
        assert_eq!(retry_delays(ProviderKind::Local), RETRY_DELAYS);
    }

    #[test]
    fn retries_on_timeout_429_and_5xx_only() {
        assert!(is_retryable(&Err(TransportError::Timeout)));
        assert!(is_retryable(&status(429)));
        assert!(is_retryable(&status(500)));
        assert!(is_retryable(&status(503)));
        assert!(is_retryable(&status(599)));
        for code in [200, 400, 401, 403, 404, 418, 499] {
            assert!(!is_retryable(&status(code)), "{code}");
        }
        assert!(!is_retryable(&Err(TransportError::Offline)));
        assert!(!is_retryable(&Err(TransportError::Tls("x".into()))));
        assert!(!is_retryable(&Err(TransportError::Unrecorded {
            hash: "h".into(),
            path: "/p".into()
        })));
    }
}
