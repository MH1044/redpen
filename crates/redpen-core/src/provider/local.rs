//! The local provider (docs/PUBLIK-INTEGRATION.md §3): Ollama, or any OpenAI-compatible server,
//! on this computer. Its address must name `localhost`, `127.0.0.1` or `::1`, checked before any
//! request, so a local call never contacts another host (invariant 12). Ollama is found with
//! `GET /api/tags` on the base URL's scheme, host and port, and [`pick_local_model`] chooses the
//! largest chat model at or under 15B parameters.

use serde::Serialize;
use serde_json::Value;

use crate::net::url_host;

/// Ollama's OpenAI-compatible address, the local provider's default.
pub const DEFAULT_LOCAL_BASE_URL: &str = "http://localhost:11434/v1";

/// The only hosts a local provider may name.
pub const LOCAL_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// The largest model [`pick_local_model`] chooses: 15B parameters, so the 14B class (which
/// Ollama reports as, for example, 14.8B) is included.
pub const MAX_LOCAL_PARAMETERS: u64 = 15_000_000_000;

/// Whether `url` is an `http` or `https` address on this computer.
pub fn is_local_url(url: &str) -> bool {
    url_host(url).is_some_and(|host| LOCAL_HOSTS.contains(&host.as_str()))
}

/// `GET /api/tags` on the same scheme, host and port as `base_url`; `None` unless `base_url`
/// is local.
pub fn tags_url(base_url: &str) -> Option<String> {
    if !is_local_url(base_url) {
        return None;
    }
    let (scheme, rest) = base_url.split_once("://")?;
    let authority = rest.split(['/', '\\', '?', '#']).next()?;
    Some(format!("{scheme}://{authority}/api/tags"))
}

/// One model Ollama has installed, as Settings lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalModel {
    pub name: String,
    pub parameter_size: String,
}

/// Every model in an `/api/tags` answer, in Ollama's order.
pub fn local_models(tags: &Value) -> Vec<LocalModel> {
    tags.get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|m| {
                    let name = m.get("name").and_then(Value::as_str)?;
                    let size = m
                        .pointer("/details/parameter_size")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    Some(LocalModel {
                        name: name.to_string(),
                        parameter_size: size.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The chat model to use from an `/api/tags` answer: names containing `embed` are dropped
/// (embedding models cannot chat), then the largest `details.parameter_size` at or under 15B
/// wins, ties going to the first name in alphabetical order. A model whose size cannot be read
/// is skipped. `None` when nothing is left.
pub fn pick_local_model(tags: &Value) -> Option<String> {
    let mut candidates: Vec<(u64, String)> = local_models(tags)
        .into_iter()
        .filter(|m| !m.name.to_ascii_lowercase().contains("embed"))
        .filter_map(|m| Some((parameter_count(&m.parameter_size)?, m.name)))
        .filter(|(count, _)| *count <= MAX_LOCAL_PARAMETERS)
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    candidates.into_iter().next().map(|(_, name)| name)
}

/// The number of parameters in Ollama's `parameter_size` (`3.2B`, `14.8B`, `137M`, `7B`), in
/// integer arithmetic so `14.8B` is exactly 14,800,000,000. `None` for anything else.
pub fn parameter_count(size: &str) -> Option<u64> {
    let size = size.trim();
    let unit = size.chars().last()?;
    let multiplier: u64 = match unit.to_ascii_uppercase() {
        'K' => 1_000,
        'M' => 1_000_000,
        'B' => 1_000_000_000,
        'T' => 1_000_000_000_000,
        _ => return None,
    };
    let number = &size[..size.len() - unit.len_utf8()];
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) || fraction.len() > 9 {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let mut count = whole.checked_mul(multiplier)?;
    if !fraction.is_empty() {
        let digits: u64 = fraction.parse().ok()?;
        let scale = 10u64.pow(u32::try_from(fraction.len()).ok()?);
        count = count.checked_add(digits.checked_mul(multiplier)? / scale)?;
    }
    Some(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tags(models: &[(&str, &str)]) -> Value {
        json!({"models": models.iter().map(|(name, size)| json!({
            "name": name, "model": name, "details": {"parameter_size": size}
        })).collect::<Vec<_>>()})
    }

    #[test]
    fn only_this_computer_is_local() {
        for url in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:11434/v1",
            "https://LOCALHOST/v1",
        ] {
            assert!(is_local_url(url), "{url}");
        }
        for url in [
            "http://192.168.1.5:11434/v1",
            "http://localhost.example.com/v1",
            "http://localhost@evil.example/v1",
            "http://127.0.0.2/v1",
            "http://0.0.0.0:11434/v1",
            "file://localhost/v1",
            "",
        ] {
            assert!(!is_local_url(url), "{url}");
        }
    }

    #[test]
    fn tags_are_on_the_base_urls_scheme_host_and_port() {
        assert_eq!(
            tags_url("http://localhost:11434/v1").as_deref(),
            Some("http://localhost:11434/api/tags")
        );
        assert_eq!(
            tags_url("http://[::1]:8080/v1/").as_deref(),
            Some("http://[::1]:8080/api/tags")
        );
        assert_eq!(tags_url("http://192.168.1.5:11434/v1"), None);
    }

    #[test]
    fn parameter_sizes_read_exactly() {
        assert_eq!(parameter_count("3.2B"), Some(3_200_000_000));
        assert_eq!(parameter_count("14.8B"), Some(14_800_000_000));
        assert_eq!(parameter_count("16.0B"), Some(16_000_000_000));
        assert_eq!(parameter_count("137M"), Some(137_000_000));
        assert_eq!(parameter_count("141B"), Some(141_000_000_000));
        assert_eq!(parameter_count("15B"), Some(15_000_000_000));
        assert_eq!(parameter_count("7b"), Some(7_000_000_000));
        assert_eq!(parameter_count("1.5T"), Some(1_500_000_000_000));
        for bad in ["", "B", "7", "x7B", "7.B.5", "-1B", "7.5Q", ".5B"] {
            assert_eq!(parameter_count(bad), None, "{bad}");
        }
    }

    #[test]
    fn picks_the_largest_chat_model_at_or_under_15b() {
        let t = tags(&[
            ("llama3.2:3b", "3.2B"),
            ("qwen2.5:14b", "14.8B"),
            ("example-16b", "16.0B"),
            ("nomic-embed-text", "137M"),
            ("mixtral:8x22b", "141B"),
        ]);
        assert_eq!(pick_local_model(&t).as_deref(), Some("qwen2.5:14b"));
        assert_eq!(
            pick_local_model(&tags(&[("nomic-embed-text", "137M")])),
            None
        );
        assert_eq!(pick_local_model(&json!({"models": []})), None);
        assert_eq!(pick_local_model(&json!({})), None);
        let ties = tags(&[("qwen2.5:7b", "7.6B"), ("other:7b", "7.6B")]);
        assert_eq!(pick_local_model(&ties).as_deref(), Some("other:7b"));
        let exactly = tags(&[("a", "15B"), ("b", "15.1B")]);
        assert_eq!(pick_local_model(&exactly).as_deref(), Some("a"));
        let unreadable = tags(&[("odd", "big"), ("small", "1B")]);
        assert_eq!(pick_local_model(&unreadable).as_deref(), Some("small"));
    }

    #[test]
    fn lists_every_model_with_its_size() {
        let t = tags(&[("llama3.2:3b", "3.2B"), ("nomic-embed-text", "137M")]);
        assert_eq!(
            local_models(&t),
            vec![
                LocalModel {
                    name: "llama3.2:3b".into(),
                    parameter_size: "3.2B".into()
                },
                LocalModel {
                    name: "nomic-embed-text".into(),
                    parameter_size: "137M".into()
                },
            ]
        );
    }
}
