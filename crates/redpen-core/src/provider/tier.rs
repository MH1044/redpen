//! The publik API tier of each call (docs/PUBLIK-INTEGRATION.md §1, rule A6; contract §1, test
//! 13): `publik-fast` for Fix grammar; `publik-balanced` for Clearer, Shorter, More formal,
//! Friendlier and Custom; `publik-smart` for those five only when Settings → Providers → "Rewrite
//! quality" is Best and the computer is linked to a publik account. While the computer is
//! anonymous, `publik-smart` falls back to `publik-balanced`.
//!
//! The rule lives in the core because `redpen-core` cannot depend on the app crate; the app's
//! `publik.rs` builds the [`TierContext`] from Settings and the publik status. Chat on publik API
//! always sends one of these tier names, never a vendor model name.

use serde::{Deserialize, Serialize};

use super::Feature;

pub use super::cache::idempotency_key;
pub use crate::store::settings::RewriteQuality;

/// The cheapest tier: cleanup and short judgements.
pub const PUBLIK_FAST: &str = "publik-fast";
/// The default tier.
pub const PUBLIK_BALANCED: &str = "publik-balanced";
/// The hardest reasoning; only for a computer linked to a publik account.
pub const PUBLIK_SMART: &str = "publik-smart";
/// Every tier name a chat call may carry.
pub const TIERS: [&str; 3] = [PUBLIK_FAST, PUBLIK_BALANCED, PUBLIK_SMART];

/// Whether this computer is linked to a publik account (`claim_state` in the credential, the
/// status headers and `GET /wallet`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimState {
    #[default]
    Anonymous,
    Claimed,
}

impl ClaimState {
    /// `anonymous` or `claimed`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ClaimState::Anonymous => "anonymous",
            ClaimState::Claimed => "claimed",
        }
    }

    /// Reads a `claim_state` value, ignoring case and surrounding space; anything else is not a
    /// claim state.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "anonymous" => Some(ClaimState::Anonymous),
            "claimed" => Some(ClaimState::Claimed),
            _ => None,
        }
    }
}

/// What the tier rule needs to know beyond the feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TierContext {
    /// Settings → Providers → Rewrite quality.
    pub quality: RewriteQuality,
    /// Whether the computer is linked, from the publik status.
    pub claim_state: ClaimState,
}

/// The tier a call for `f` sends on publik API. A provider's Test (and the maintainer's proof
/// call) uses the cheapest tier.
pub fn publik_tier(f: Feature, ctx: &TierContext) -> &'static str {
    match f {
        Feature::FixGrammar | Feature::Test => PUBLIK_FAST,
        Feature::Clearer
        | Feature::Shorter
        | Feature::MoreFormal
        | Feature::Friendlier
        | Feature::Custom => {
            if ctx.quality == RewriteQuality::Best && ctx.claim_state == ClaimState::Claimed {
                PUBLIK_SMART
            } else {
                PUBLIK_BALANCED
            }
        }
    }
}

/// Whether `model` is a publik tier name.
pub fn is_tier(model: &str) -> bool {
    TIERS.contains(&model)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(quality: RewriteQuality, claim_state: ClaimState) -> TierContext {
        TierContext {
            quality,
            claim_state,
        }
    }

    #[test]
    fn the_tier_table_of_publik_integration_section_1() {
        use ClaimState::*;
        use RewriteQuality::*;
        let five = [
            Feature::Clearer,
            Feature::Shorter,
            Feature::MoreFormal,
            Feature::Friendlier,
            Feature::Custom,
        ];
        for f in five {
            assert_eq!(publik_tier(f, &ctx(Standard, Anonymous)), "publik-balanced");
            assert_eq!(publik_tier(f, &ctx(Standard, Claimed)), "publik-balanced");
            // Smart falls back to balanced while the computer is anonymous.
            assert_eq!(publik_tier(f, &ctx(Best, Anonymous)), "publik-balanced");
            assert_eq!(publik_tier(f, &ctx(Best, Claimed)), "publik-smart");
        }
        for c in [ctx(Standard, Anonymous), ctx(Best, Claimed)] {
            assert_eq!(publik_tier(Feature::FixGrammar, &c), "publik-fast");
            assert_eq!(publik_tier(Feature::Test, &c), "publik-fast");
        }
    }

    #[test]
    fn every_tier_is_a_tier_name() {
        let all = [
            ctx(RewriteQuality::Best, ClaimState::Claimed),
            TierContext::default(),
        ];
        for c in all {
            for f in [Feature::FixGrammar, Feature::Clearer, Feature::Custom] {
                assert!(is_tier(publik_tier(f, &c)));
            }
        }
        assert!(!is_tier("gpt-4o"));
        assert!(!is_tier(""));
    }

    #[test]
    fn the_default_context_is_standard_and_anonymous() {
        assert_eq!(
            TierContext::default(),
            ctx(RewriteQuality::Standard, ClaimState::Anonymous)
        );
    }

    #[test]
    fn claim_states_read_and_write_their_names() {
        assert_eq!(ClaimState::parse("claimed"), Some(ClaimState::Claimed));
        assert_eq!(
            ClaimState::parse(" Anonymous "),
            Some(ClaimState::Anonymous)
        );
        assert_eq!(ClaimState::parse("linked"), None);
        assert_eq!(
            serde_json::to_value(ClaimState::Claimed).unwrap(),
            serde_json::json!("claimed")
        );
        assert_eq!(ClaimState::Anonymous.as_str(), "anonymous");
    }
}
