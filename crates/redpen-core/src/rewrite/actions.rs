//! The action table (docs/PUBLIK-INTEGRATION.md §6): for each of the six rewrite actions, its
//! prompt file, its tier, temperature, `max_tokens` formula and schema (from the prompt's front
//! matter, through `provider::tier::publik_tier` for the tier), what its cache input holds, and
//! which checks its answer gets.
//!
//! | Action | Prompt | Tier | Temp. | `max_tokens` | Schema | Checks after the schema |
//! |---|---|---|---|---|---|---|
//! | Fix grammar | `fix-grammar.v1` | `publik-fast` | 0 | 2w + 100 | `rewrite-result.v1` | markers, cleaning, unchanged, language, guard |
//! | Clearer | `clearer.v1` | balanced (smart: Best and linked) | 0.4 | 2w + 100 | `rewrite-result.v1` | markers, cleaning, unchanged, language |
//! | Shorter | `shorter.v1` | as Clearer | 0.4 | 2w + 100 | `rewrite-result.v1` | as Clearer, then "not shorter" |
//! | More formal | `more-formal.v1` | as Clearer | 0.4 | 2w + 100 | `rewrite-result.v1` | as Clearer |
//! | Friendlier | `friendlier.v1` | as Clearer | 0.4 | 2w + 100 | `rewrite-result.v1` | as Clearer |
//! | Custom | `custom.v1` | as Clearer | 0.4 | 2w + 150 | `custom-result.v1` | as Clearer; language only when `changes_language` is false |
//!
//! (w: [`crate::words::words`] of the text as sent; each `max_tokens` is clamped to 256..4096.)

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::prompts::{self, Prompt};
use crate::check::Dialect;
use crate::provider::Feature;

/// A rewrite action, one per popup button. Named in kebab case (`fix-grammar`, `more-formal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    FixGrammar,
    Clearer,
    Shorter,
    MoreFormal,
    Friendlier,
    Custom,
}

impl Action {
    /// Every action, in the popup's order.
    pub const ALL: [Action; 6] = [
        Action::FixGrammar,
        Action::Clearer,
        Action::Shorter,
        Action::MoreFormal,
        Action::Friendlier,
        Action::Custom,
    ];

    /// The kebab-case name, which is also the prompt's id.
    pub fn as_str(self) -> &'static str {
        match self {
            Action::FixGrammar => "fix-grammar",
            Action::Clearer => "clearer",
            Action::Shorter => "shorter",
            Action::MoreFormal => "more-formal",
            Action::Friendlier => "friendlier",
            Action::Custom => "custom",
        }
    }

    /// The action named by [`Self::as_str`], or `None`.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == name)
    }

    /// The feature its calls are counted and tiered under.
    pub fn feature(self) -> Feature {
        match self {
            Action::FixGrammar => Feature::FixGrammar,
            Action::Clearer => Feature::Clearer,
            Action::Shorter => Feature::Shorter,
            Action::MoreFormal => Feature::MoreFormal,
            Action::Friendlier => Feature::Friendlier,
            Action::Custom => Feature::Custom,
        }
    }

    /// The prompt file it sends: `<id>.v1.md` for every action in this version.
    pub fn prompt_file(self) -> &'static str {
        match self {
            Action::FixGrammar => "fix-grammar.v1.md",
            Action::Clearer => "clearer.v1.md",
            Action::Shorter => "shorter.v1.md",
            Action::MoreFormal => "more-formal.v1.md",
            Action::Friendlier => "friendlier.v1.md",
            Action::Custom => "custom.v1.md",
        }
    }

    /// The loaded prompt. The embedded prompts are checked by this module's tests, so a build
    /// whose prompt does not load never ships.
    pub fn prompt(self) -> &'static Prompt {
        prompts::get(self.prompt_file()).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Whether the prompt takes `{style}` and the cache input holds `style` and `variant`. Fix
    /// grammar keeps the author's style by definition and, at temperature 0, has no other
    /// version.
    pub fn uses_style(self) -> bool {
        self != Action::FixGrammar
    }

    /// Whether **Another version** applies, so `variant` is part of the cache key.
    pub fn has_variant(self) -> bool {
        self != Action::FixGrammar
    }

    /// Whether the prompt takes a typed `{instruction}`.
    pub fn uses_instruction(self) -> bool {
        self == Action::Custom
    }

    /// Whether the minimal-edit guard checks its answer (ADR-0004).
    pub fn guarded(self) -> bool {
        self == Action::FixGrammar
    }

    /// Whether its answer is warned about when it is not shorter.
    pub fn must_be_shorter(self) -> bool {
        self == Action::Shorter
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An action name that is not one of the six.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "Unknown action \"{0}\". Use fix-grammar, clearer, shorter, more-formal, friendlier or custom."
)]
pub struct UnknownAction(pub String);

impl FromStr for Action {
    type Err = UnknownAction;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| UnknownAction(s.to_string()))
    }
}

/// A style preset (`{style}`). Only `Own` is offered until style presets arrive (module 08); the
/// others already have their sentences from docs/PUBLIK-INTEGRATION.md §6.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    #[default]
    Own,
    PlainEnglish,
    Academic,
    Business,
}

impl Style {
    /// The snake-case name the cache input holds: `own`, `plain_english`, `academic`,
    /// `business`.
    pub fn as_str(self) -> &'static str {
        match self {
            Style::Own => "own",
            Style::PlainEnglish => "plain_english",
            Style::Academic => "academic",
            Style::Business => "business",
        }
    }

    /// The `{style}` value.
    pub fn sentence(self) -> &'static str {
        match self {
            Style::Own => "Keep the author's own style.",
            Style::PlainEnglish => {
                "Plain English: short sentences, everyday words, active voice, no jargon."
            }
            Style::Academic => {
                "Academic: precise and formal, no contractions, hedged only where the text hedges."
            }
            Style::Business => {
                "Business: concise and courteous, the request or decision first, no filler."
            }
        }
    }
}

/// The `{dialect_name}` value: `British English` and so on.
pub fn dialect_name(d: Dialect) -> &'static str {
    match d {
        Dialect::American => "American English",
        Dialect::British => "British English",
        Dialect::Australian => "Australian English",
        Dialect::Canadian => "Canadian English",
        Dialect::Indian => "Indian English",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::tier::{TierContext, publik_tier};

    #[test]
    fn the_table_matches_section_6() {
        let expected = [
            (
                Action::FixGrammar,
                "publik-fast",
                0.0,
                100,
                "rewrite-result.v1",
            ),
            (
                Action::Clearer,
                "publik-balanced",
                0.4,
                100,
                "rewrite-result.v1",
            ),
            (
                Action::Shorter,
                "publik-balanced",
                0.4,
                100,
                "rewrite-result.v1",
            ),
            (
                Action::MoreFormal,
                "publik-balanced",
                0.4,
                100,
                "rewrite-result.v1",
            ),
            (
                Action::Friendlier,
                "publik-balanced",
                0.4,
                100,
                "rewrite-result.v1",
            ),
            (
                Action::Custom,
                "publik-balanced",
                0.4,
                150,
                "custom-result.v1",
            ),
        ];
        for (action, tier, temperature, base, schema) in expected {
            let p = action.prompt();
            assert_eq!(p.id, action.as_str());
            assert_eq!(p.version, 1);
            assert_eq!(p.file, action.prompt_file());
            assert_eq!(p.tier, tier, "{action}");
            assert_eq!(publik_tier(action.feature(), &TierContext::default()), tier);
            assert!((p.temperature - temperature).abs() < 1e-6, "{action}");
            assert_eq!(
                p.max_tokens.to_string(),
                format!("clamp(2 * words(text) + {base}, 256, 4096)")
            );
            assert_eq!(p.schema, schema);
            assert_eq!(p.user.contains("{instruction}"), action.uses_instruction());
            assert_eq!(p.system.contains("{style}"), action.uses_style());
            assert!(p.system.contains("{dialect_name}"));
        }
    }

    #[test]
    fn names_parse_both_ways() {
        for a in Action::ALL {
            assert_eq!(a.as_str().parse::<Action>().unwrap(), a);
            assert_eq!(
                serde_json::to_value(a).unwrap(),
                serde_json::json!(a.as_str())
            );
            assert_eq!(a.feature().as_str(), a.as_str());
        }
        assert_eq!(
            "rephrase".parse::<Action>().unwrap_err().to_string(),
            "Unknown action \"rephrase\". Use fix-grammar, clearer, shorter, more-formal, friendlier or custom."
        );
    }

    #[test]
    fn styles_and_dialects_have_their_section_6_values() {
        assert_eq!(Style::default(), Style::Own);
        assert_eq!(Style::Own.sentence(), "Keep the author's own style.");
        assert_eq!(Style::PlainEnglish.as_str(), "plain_english");
        assert_eq!(
            serde_json::to_value(Style::PlainEnglish).unwrap(),
            serde_json::json!("plain_english")
        );
        let names: Vec<&str> = Dialect::ALL.iter().map(|d| dialect_name(*d)).collect();
        assert_eq!(
            names,
            [
                "American English",
                "British English",
                "Australian English",
                "Canadian English",
                "Indian English"
            ]
        );
    }
}
