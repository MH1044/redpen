//! `RedpenResolver`: the provider for chat calls, keyed on Settings → Providers → AI provider
//! (docs/PUBLIK-INTEGRATION.md §3), in the contract's order (§4).
//!
//! - **Local**: the local server from Settings; publik API is never consulted, so a publik
//!   credential on disk is never used and its balance never spent (contract test 12).
//! - **Your own key**: `resolve_provider(Some(own key))` when the key, address and model are
//!   saved; otherwise `NotConfigured`. A choice of the own key never falls through to publik
//!   API, so nothing is sent to a provider the user did not choose.
//! - **publik API**, or no choice yet: `resolve_provider(None)`, which tries `PUBLIK_API_KEY`,
//!   then the credential file, then gives `NeedsConsent` (the consent view).
//!
//! On publik API the provider's `chat_model` is `publik-balanced`, the default tier; a feature's
//! call sends `publik_tier(feature, &resolver.tier_context())`, whose claim state comes from the
//! publik status (or the credential before any status exists).

use std::path::PathBuf;
use std::sync::Arc;

use redpen_core::provider::tier::PUBLIK_BALANCED;
use redpen_core::provider::{AiError, ProviderResolver, ResolvedProvider, TierContext};
use redpen_core::store::Secrets;
use redpen_core::store::settings::ChatChoice;
use secrecy::ExposeSecret;

use crate::publik::{OwnKey, PublikOptions, Resolution, SharedSettings};
use crate::status::StatusStore;

/// The resolver over Settings, `secrets.json`, the publik credential and the publik status.
pub struct RedpenResolver {
    settings: SharedSettings,
    config_dir: PathBuf,
    options: PublikOptions,
    status: Arc<StatusStore>,
}

impl std::fmt::Debug for RedpenResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedpenResolver").finish_non_exhaustive()
    }
}

impl RedpenResolver {
    /// A resolver reading the own key from `<config_dir>/secrets.json` and publik API through
    /// `options`.
    pub fn new(
        settings: SharedSettings,
        config_dir: PathBuf,
        options: PublikOptions,
        status: Arc<StatusStore>,
    ) -> Self {
        Self {
            settings,
            config_dir,
            options,
            status,
        }
    }

    /// The saved own key, when its key, address and model are all there.
    fn own_key(&self, base_url: &str, chat_model: &str) -> Option<OwnKey> {
        let secrets = Secrets::load(&self.config_dir).ok()?;
        let key = secrets
            .own_key
            .filter(|k| !k.expose_secret().trim().is_empty())?;
        let (base_url, chat_model) = (base_url.trim(), chat_model.trim());
        if base_url.is_empty() || chat_model.is_empty() {
            return None;
        }
        Some(OwnKey {
            base_url: base_url.to_string(),
            key,
            chat_model: chat_model.to_string(),
        })
    }
}

impl ProviderResolver for RedpenResolver {
    fn chat(&self) -> Result<ResolvedProvider, AiError> {
        let providers = self
            .settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .providers
            .clone();
        match providers.chat {
            Some(ChatChoice::Local) => Ok(ResolvedProvider::from_local_settings(&providers.local)),
            Some(ChatChoice::OwnKey) => {
                let own = self
                    .own_key(&providers.own_key.base_url, &providers.own_key.chat_model)
                    .ok_or(AiError::NotConfigured)?;
                match self.options.resolve(Some(&own)) {
                    Resolution::OwnKey(own) => Ok(own.provider()),
                    _ => Err(AiError::NotConfigured),
                }
            }
            Some(ChatChoice::Publik) | None => match self.options.resolve(None) {
                Resolution::Env(c) | Resolution::File(c) => Ok(c.provider(PUBLIK_BALANCED)),
                Resolution::OwnKey(_) | Resolution::NeedsConsent => Err(AiError::NeedsConsent),
            },
        }
    }

    fn tier_context(&self) -> TierContext {
        let quality = self
            .settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .providers
            .rewrite_quality;
        let claim_state = self
            .status
            .current()
            .map(|s| s.claim_state)
            .or_else(|| {
                self.options
                    .resolve(None)
                    .credential()
                    .map(|c| c.claim_state)
            })
            .unwrap_or_default();
        TierContext {
            quality,
            claim_state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publik::{Credential, PublikStatus};
    use redpen_core::clock::FakeClock;
    use redpen_core::provider::{ClaimState, ProviderKind, RewriteQuality, SecretString};
    use redpen_core::store::Settings;
    use std::sync::RwLock;

    struct Fixture {
        _dir: tempfile::TempDir,
        config: PathBuf,
        settings: SharedSettings,
        options: PublikOptions,
        status: Arc<StatusStore>,
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn env_key(name: &str) -> Option<String> {
        match name {
            "PUBLIK_API_KEY" => Some("pk_live_env".into()),
            _ => None,
        }
    }

    fn fixture(chat: Option<ChatChoice>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let mut s = Settings::default();
        s.providers.chat = chat;
        s.providers.own_key.base_url = "https://api.example.com/v1".into();
        s.providers.own_key.chat_model = "example-chat".into();
        let options = PublikOptions {
            credential_path: dir.path().join("publik").join("redpen.json"),
            app_token: None,
            base_url: "https://publikhq.com/api/v1".into(),
            app_version: "0.1.0".into(),
            new_install_id: || "00000000-0000-4000-8000-000000000000".into(),
            env: no_env,
        };
        let status = Arc::new(StatusStore::new(
            &dir.path().join("data"),
            Arc::new(FakeClock::at(0)),
        ));
        Fixture {
            config,
            settings: Arc::new(RwLock::new(s)),
            options,
            status,
            _dir: dir,
        }
    }

    fn resolver(f: &Fixture) -> RedpenResolver {
        RedpenResolver::new(
            f.settings.clone(),
            f.config.clone(),
            f.options.clone(),
            f.status.clone(),
        )
    }

    fn write_credential(f: &Fixture, claim_state: ClaimState) {
        Credential {
            key: Some(SecretString::from("pk_live_a")),
            install_id: "5f0c8a52-6e0b-4c55-9a3e-2a3f1d9b7c11".into(),
            claim_state,
            ..Credential::default()
        }
        .save(&f.options.credential_path)
        .unwrap();
    }

    fn save_own_key(f: &Fixture) {
        Secrets {
            own_key: Some(SecretString::from("sk-test-a")),
            search_key: None,
        }
        .save(&f.config)
        .unwrap();
    }

    #[test]
    fn local_never_consults_publik() {
        let f = fixture(Some(ChatChoice::Local));
        write_credential(&f, ClaimState::Claimed);
        let p = resolver(&f).chat().unwrap();
        assert_eq!(p.kind, ProviderKind::Local);
        assert_eq!(p.base_url, "http://localhost:11434/v1");
        assert!(p.key.is_none());
    }

    #[test]
    fn the_own_key_when_chosen_and_saved() {
        let f = fixture(Some(ChatChoice::OwnKey));
        write_credential(&f, ClaimState::Anonymous);
        assert_eq!(resolver(&f).chat().unwrap_err(), AiError::NotConfigured);
        save_own_key(&f);
        let p = resolver(&f).chat().unwrap();
        assert_eq!(p.kind, ProviderKind::OwnKey);
        assert_eq!(p.chat_model, "example-chat");
        assert_eq!(p.key.unwrap().expose_secret(), "sk-test-a");
    }

    #[test]
    fn publik_or_no_choice_resolves_env_then_file_then_consent() {
        for chat in [Some(ChatChoice::Publik), None] {
            let mut f = fixture(chat);
            // An own key saved while publik API is chosen stays unused.
            save_own_key(&f);
            assert_eq!(resolver(&f).chat().unwrap_err(), AiError::NeedsConsent);
            write_credential(&f, ClaimState::Anonymous);
            let p = resolver(&f).chat().unwrap();
            assert_eq!(p.kind, ProviderKind::Publik);
            assert_eq!(p.chat_model, "publik-balanced");
            assert_eq!(p.key.unwrap().expose_secret(), "pk_live_a");
            f.options.env = env_key;
            let p = resolver(&f).chat().unwrap();
            assert_eq!(p.key.unwrap().expose_secret(), "pk_live_env");
        }
    }

    #[test]
    fn the_tier_context_follows_settings_and_the_status() {
        let f = fixture(Some(ChatChoice::Publik));
        assert_eq!(resolver(&f).tier_context(), TierContext::default());
        write_credential(&f, ClaimState::Claimed);
        f.settings.write().unwrap().providers.rewrite_quality = RewriteQuality::Best;
        assert_eq!(
            resolver(&f).tier_context(),
            TierContext {
                quality: RewriteQuality::Best,
                claim_state: ClaimState::Claimed
            }
        );
        // The status, once publik API has answered, wins over the credential.
        f.status
            .replace(PublikStatus::new(ClaimState::Anonymous, 0, 0));
        assert_eq!(
            resolver(&f).tier_context().claim_state,
            ClaimState::Anonymous
        );
    }
}
