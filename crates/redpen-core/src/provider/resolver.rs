//! Which provider a chat call goes to (docs/PUBLIK-INTEGRATION.md §3, contract §4).
//!
//! The app implements [`ProviderResolver`] over Settings, the saved own key and the publik
//! credential (`RedpenResolver` in `app/src-tauri/src/resolver.rs`), because the publik
//! credential is managed by the app's `publik.rs`, which the core cannot depend on. Flows ask the
//! resolver at the moment of the call, so a change in Settings takes effect on the next action.

use super::errors::AiError;
use super::resolved::ResolvedProvider;
use super::tier::TierContext;

/// Resolves the provider for chat calls.
pub trait ProviderResolver: Send + Sync {
    /// The provider a chat call goes to now. On publik API, `chat_model` is the default tier
    /// (`publik-balanced`); a feature's call sends the tier [`super::tier::publik_tier`] gives
    /// for [`ProviderResolver::tier_context`]. `NeedsConsent` when no provider is chosen and
    /// none is set up, or publik API is chosen with no key on this computer.
    fn chat(&self) -> Result<ResolvedProvider, AiError>;

    /// What the tier rule needs: Rewrite quality from Settings and whether the computer is
    /// linked. Standard and anonymous unless the implementation knows better.
    fn tier_context(&self) -> TierContext {
        TierContext::default()
    }
}

/// A resolver that always gives the same answer: tests, and the command line, which never
/// provisions publik API.
#[derive(Debug, Clone)]
pub struct FixedResolver {
    provider: Result<ResolvedProvider, AiError>,
    tier_context: TierContext,
}

impl FixedResolver {
    /// Always this provider, with the default tier context.
    pub fn new(provider: ResolvedProvider) -> Self {
        Self {
            provider: Ok(provider),
            tier_context: TierContext::default(),
        }
    }

    /// Always this error.
    pub fn failing(error: AiError) -> Self {
        Self {
            provider: Err(error),
            tier_context: TierContext::default(),
        }
    }

    /// The same resolver with this tier context.
    pub fn with_tier_context(mut self, ctx: TierContext) -> Self {
        self.tier_context = ctx;
        self
    }
}

impl ProviderResolver for FixedResolver {
    fn chat(&self) -> Result<ResolvedProvider, AiError> {
        self.provider.clone()
    }

    fn tier_context(&self) -> TierContext {
        self.tier_context
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::tier::{ClaimState, RewriteQuality};

    #[test]
    fn a_fixed_resolver_gives_its_provider_or_its_error() {
        let r = FixedResolver::new(ResolvedProvider::fake());
        assert_eq!(r.chat().unwrap().base_url, "http://fake.invalid/v1");
        assert_eq!(r.tier_context(), TierContext::default());
        let r = FixedResolver::failing(AiError::NeedsConsent);
        assert_eq!(r.chat().unwrap_err(), AiError::NeedsConsent);
        let ctx = TierContext {
            quality: RewriteQuality::Best,
            claim_state: ClaimState::Claimed,
        };
        let r = FixedResolver::new(ResolvedProvider::fake()).with_tier_context(ctx);
        assert_eq!(r.tier_context(), ctx);
        // Usable as a trait object, as the flows hold it.
        let dynamic: &dyn ProviderResolver = &r;
        assert!(dynamic.chat().is_ok());
    }
}
