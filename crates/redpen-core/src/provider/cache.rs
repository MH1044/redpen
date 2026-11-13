//! The answer cache (docs/ARCHITECTURE.md §4, docs/PUBLIK-INTEGRATION.md §5). Identical input
//! never spends twice; caches are per provider, because the provider is part of the key; a
//! prompt version bump misses on purpose.
//!
//! - **Key K**: SHA-256 (64 lowercase hex) of the canonical JSON (module 00's
//!   [`canonical_json`], the fake transport's own function) of `{"provider", "model", "prompt",
//!   "prompt_version", "input"}`.
//! - **Policies**: `Disk` stores the answer in `ai_cache` for `remember_answers_days` (at most
//!   7); `Memory` keeps it in a map for the life of the process; `Off` neither reads nor
//!   writes. `Disk` with 0 days behaves as `Memory`.
//! - Errors and invalid answers are never cached; the caller stores only a usable answer.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::resolved::ResolvedProvider;
use super::{CachePolicy, ChatCall};
use crate::net::canonical_json;

/// The longest `remember_answers_days` honoured: rewrites hold text captured from other apps, so
/// they are never kept on disk for more than a week.
pub const MAX_REMEMBER_DAYS: u32 = 7;

/// The cache key K of a call to a provider: 64 lowercase hex characters.
pub fn cache_key(p: &ResolvedProvider, c: &ChatCall) -> String {
    let input = json!({
        "provider": p.kind.as_str(),
        "model": p.chat_model,
        "prompt": c.prompt_id,
        "prompt_version": c.prompt_version,
        "input": c.cache_input,
    });
    let digest = Sha256::digest(canonical_json(&input).as_bytes());
    crate::net::hash::hex(&digest)
}

/// The `x-publik-idempotency-key` of a call with cache key K (docs/PUBLIK-INTEGRATION.md §5): the
/// first 32 characters of K for a first call and for any Try again after a 402, 401, 429, 503,
/// timeout or offline error; for the n-th Try again after an answer failed validation, the first
/// 32 hex characters of SHA-256(`<K>:<n>`), so publik API treats it as a new request rather than
/// a replay of the unusable answer. Always 32 lowercase hex characters.
pub fn idempotency_key(cache_key: &str, validation_retry: u32) -> String {
    let full = if validation_retry == 0 {
        cache_key.to_ascii_lowercase()
    } else {
        let input = format!("{cache_key}:{validation_retry}");
        crate::net::hash::hex(&Sha256::digest(input.as_bytes()))
    };
    // K is 64 hex characters; anything shorter is padded from its own hash so the header is
    // always 32 lowercase hex.
    if full.len() >= 32 && full.bytes().all(|b| b.is_ascii_hexdigit()) {
        full[..32].to_string()
    } else {
        crate::net::hash::hex(&Sha256::digest(full.as_bytes()))[..32].to_string()
    }
}

/// Where an answer for this policy goes, given the current `remember_answers_days`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Disk { days: u32 },
    Memory,
    Nowhere,
}

/// The placement for a policy and the user's setting (clamped to 7 days).
pub fn placement(policy: CachePolicy, remember_answers_days: u32) -> Placement {
    match policy {
        CachePolicy::Off => Placement::Nowhere,
        CachePolicy::Memory => Placement::Memory,
        CachePolicy::Disk => match remember_answers_days.min(MAX_REMEMBER_DAYS) {
            0 => Placement::Memory,
            days => Placement::Disk { days },
        },
    }
}

/// What `ai_cache.response_json` holds for a chat answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedAnswer {
    pub content: String,
}

impl CachedAnswer {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("an answer serialises")
    }

    pub fn from_json(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }
}

/// The process-lifetime cache of `CachePolicy::Memory`.
#[derive(Debug, Default)]
pub struct MemoryCache {
    map: Mutex<HashMap<String, CachedAnswer>>,
}

impl MemoryCache {
    pub fn get(&self, key: &str) -> Option<CachedAnswer> {
        self.lock().get(key).cloned()
    }

    pub fn put(&self, key: String, answer: CachedAnswer) {
        self.lock().insert(key, answer);
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CachedAnswer>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Feature, Message, ProviderKind};
    use std::time::Duration;

    fn call(prompt_version: u32) -> ChatCall {
        ChatCall {
            feature: Feature::FixGrammar,
            prompt_id: "fix-grammar",
            prompt_version,
            messages: vec![Message::user("hi")],
            max_tokens: 256,
            temperature: 0.0,
            timeout: Duration::from_secs(30),
            cache_input: json!({"text": "hi", "dialect": "british"}),
            cache: CachePolicy::Disk,
            validation_retry: 0,
        }
    }

    #[test]
    fn the_key_is_sha256_of_the_canonical_input() {
        let p =
            ResolvedProvider::own_key("https://api.example.com/v1", "sk-test-a", "example-chat");
        // The canonical string, written out by hand from the spec: keys sorted, no whitespace.
        let canonical = r#"{"input":{"dialect":"british","text":"hi"},"model":"example-chat","prompt":"fix-grammar","prompt_version":1,"provider":"own_key"}"#;
        let expected = crate::net::hash::hex(&Sha256::digest(canonical.as_bytes()));
        let key = cache_key(&p, &call(1));
        assert_eq!(key, expected);
        assert_eq!(key.len(), 64);
        assert!(
            key.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn the_key_changes_with_provider_model_prompt_version_and_input_but_not_the_key() {
        let p =
            ResolvedProvider::own_key("https://api.example.com/v1", "sk-test-a", "example-chat");
        let base = cache_key(&p, &call(1));
        let mut local = p.clone();
        local.kind = ProviderKind::Local;
        assert_ne!(cache_key(&local, &call(1)), base);
        let mut other_model = p.clone();
        other_model.chat_model = "other".into();
        assert_ne!(cache_key(&other_model, &call(1)), base);
        assert_ne!(cache_key(&p, &call(2)), base);
        let mut other_input = call(1);
        other_input.cache_input = json!({"text": "ho", "dialect": "british"});
        assert_ne!(cache_key(&p, &other_input), base);
        let other_key =
            ResolvedProvider::own_key("https://api.example.com/v1", "sk-test-b", "example-chat");
        assert_eq!(cache_key(&other_key, &call(1)), base);
        let mut other_retry = call(1);
        other_retry.validation_retry = 3;
        assert_eq!(cache_key(&p, &other_retry), base);
    }

    #[test]
    fn placements_follow_the_policy_and_the_setting() {
        assert_eq!(placement(CachePolicy::Disk, 7), Placement::Disk { days: 7 });
        assert_eq!(
            placement(CachePolicy::Disk, 30),
            Placement::Disk { days: 7 }
        );
        assert_eq!(placement(CachePolicy::Disk, 3), Placement::Disk { days: 3 });
        assert_eq!(placement(CachePolicy::Disk, 0), Placement::Memory);
        assert_eq!(placement(CachePolicy::Memory, 7), Placement::Memory);
        assert_eq!(placement(CachePolicy::Off, 7), Placement::Nowhere);
    }

    #[test]
    fn memory_cache_round_trips_and_clears() {
        let m = MemoryCache::default();
        assert!(m.is_empty());
        m.put(
            "k".into(),
            CachedAnswer {
                content: "hello".into(),
            },
        );
        assert_eq!(m.get("k").unwrap().content, "hello");
        assert_eq!(m.len(), 1);
        m.clear();
        assert!(m.get("k").is_none());
    }

    #[test]
    fn idempotency_keys_match_sha256sum() {
        // K from module 00 test 2; the retry keys are `sha256sum` of "K:1" and "K:2", cut to 32.
        let k = "22f53c07ac6b84e42b6fea37d9f1ce05a6c42780307b5932ccf2031f7ef66dd1";
        assert_eq!(idempotency_key(k, 0), "22f53c07ac6b84e42b6fea37d9f1ce05");
        assert_eq!(idempotency_key(k, 1), "b6058582879f1920e30934ad9402c4db");
        assert_eq!(idempotency_key(k, 2), "84491dd9d4f881c8ec68ba4c829a264a");
        for n in 0..5 {
            let key = idempotency_key(k, n);
            assert_eq!(key.len(), 32);
            assert!(
                key.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            );
        }
        // Never anything but 32 lowercase hex, even for a malformed K.
        let odd = idempotency_key("short", 0);
        assert_eq!(odd.len(), 32);
        assert!(odd.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            idempotency_key(&k.to_ascii_uppercase(), 0),
            idempotency_key(k, 0)
        );
    }

    #[test]
    fn cached_answers_are_json() {
        let a = CachedAnswer {
            content: "hé \"x\"".into(),
        };
        assert_eq!(a.to_json(), r#"{"content":"hé \"x\""}"#);
        assert_eq!(CachedAnswer::from_json(&a.to_json()), Some(a));
        assert_eq!(CachedAnswer::from_json("not json"), None);
    }
}
