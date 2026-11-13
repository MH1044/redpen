//! The rewrite engine (docs/modules/04-rewrite-engine.md): given a selected text and an action,
//! build the exact request of docs/PUBLIK-INTEGRATION.md §6, send it through [`AiClient`], and
//! return a validated, restored, cleaned result with a word diff and any warnings, or a clear
//! refusal. Nothing invalid is ever returned as a result (invariant 8).
//!
//! A rewrite, in order:
//!
//! 1. **Limits**, before any request: an empty or whitespace-only text is `Empty`; over
//!    [`MAX_WORDS`] words is `TooLong`; Custom needs an instruction of at most
//!    [`MAX_INSTRUCTION_CHARS`] characters.
//! 2. **Redaction** when on ([`crate::redact`]): the text, and Custom's instruction, with one
//!    numbering. A text that already holds a marker is `RedactionCollision`.
//! 3. **The request**: the action's prompt ([`actions`], [`prompts`]) rendered with the dialect's
//!    name, the style (not for Fix grammar), the text exactly as selected (its leading and
//!    trailing whitespace included), and the instruction trimmed; `max_tokens` from the prompt's
//!    formula over the text as sent; the model is the publik tier from `publik_tier` on publik
//!    API and the fake provider, and the configured model on the own key and a local server.
//! 4. **The cache key** input is `{text, dialect, style, instruction?, variant}` with the text as
//!    sent (after redaction), so the same text with other private details shares an answer;
//!    Fix grammar's is `{text, dialect}`. `variant` counts presses of Another version.
//! 5. **Validation**, inside the client so an answer that fails is never cached: the schema
//!    ([`parse`]); closing tags turned back ([`prompts::unescape`]); with redaction on, every
//!    marker sent in the text is in the answer and no other marker is (`PlaceholderDropped`,
//!    `PlaceholderInvented`); the markers restored; cleaning ([`clean`]). An answer left empty by
//!    cleaning is `InvalidAnswer`.
//! 6. **The outcome**: identical to the original is "No changes needed." (`unchanged`, no
//!    warnings); otherwise language keeping ([`crate::lang`]; skipped for Custom when
//!    `changes_language` is true), the guard for Fix grammar and "not shorter" for Shorter
//!    ([`guard`]), and the word diff ([`crate::diff`]). A language or guard warning makes the
//!    result not the default action (`accept_default: false`).
//!
//! **Validation retries.** After an answer failed validation, the next Try again for the same
//! input sends `validation_retry` n, the number of failed answers for that cache key so far, so
//! publik API gets the idempotency key of SHA-256(`<K>:<n>`) and treats it as a new request; a
//! valid answer is cached under K itself and resets the count ([`ValidationRetries`]). A Try
//! again after any other error sends the same key as before.

pub mod actions;
pub mod clean;
pub mod guard;
pub mod parse;
pub mod prompts;

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::{Value, json};

pub use self::actions::{Action, Style, UnknownAction, dialect_name};
pub use self::guard::{GUARD_MIN_CHANGED, GUARD_RATIO_PERCENT};
pub use crate::diff::{DiffOp, edit_ratio, word_diff};
pub use crate::redact::{Entity, EntityKind, redact, restore};

use crate::check::Dialect;
use crate::provider::client::chat_body;
use crate::provider::tier::{TierContext, publik_tier};
use crate::provider::{
    AiClient, AiError, CachePolicy, ChatCall, ChatReply, Message, ProviderKind, ResolvedProvider,
    chat_timeout,
};
use crate::redact::{MARKER, markers_in};
use crate::words::{group_thousands, words};

/// The most words a selection may have for a rewrite.
pub const MAX_WORDS: usize = 1_500;

/// The most characters a Custom instruction may have.
pub const MAX_INSTRUCTION_CHARS: usize = 300;

/// What the popup shows for an `unchanged` outcome.
pub const NO_CHANGES: &str = "No changes needed.";

/// One rewrite to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteRequest {
    pub action: Action,
    /// The selection, exactly as captured.
    pub text: String,
    /// Custom's typed instruction; the other actions ignore it.
    pub instruction: Option<String>,
    pub dialect: Dialect,
    /// Only `Own` until style presets arrive (module 08). Fix grammar ignores it.
    pub style: Style,
    /// Settings → Privacy → redaction (on by default).
    pub redact: bool,
    /// Presses of Another version after a success; Fix grammar ignores it.
    pub variant: u32,
    /// Answers for this input that failed validation so far. Set by [`ValidationRetries`],
    /// never by the interface.
    pub validation_retry: u32,
}

impl RewriteRequest {
    /// A first request for `action` on `text`: American English, the author's own style,
    /// redaction on, no instruction, variant 0.
    pub fn new(action: Action, text: impl Into<String>) -> Self {
        Self {
            action,
            text: text.into(),
            instruction: None,
            dialect: Dialect::American,
            style: Style::Own,
            redact: true,
            variant: 0,
            validation_retry: 0,
        }
    }
}

/// A warning shown above a result. `Display` is its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// Fix grammar changed more than a grammar fix should (ADR-0004).
    Guard { changed: usize, total: usize },
    /// The answer is reliably in another language than the text.
    Language { expected: String, got: String },
    /// Shorter's answer has at least as many words as the text.
    NotShorter,
}

impl Warning {
    /// `guard`, `language` or `not_shorter`.
    pub fn kind(&self) -> &'static str {
        match self {
            Warning::Guard { .. } => "guard",
            Warning::Language { .. } => "language",
            Warning::NotShorter => "not_shorter",
        }
    }

    /// Whether the result stops being the default action, so Enter does not accept it.
    pub fn blocks_default(&self) -> bool {
        matches!(self, Warning::Guard { .. } | Warning::Language { .. })
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Warning::Guard { changed, total } => write!(
                f,
                "This changes more than a grammar fix should ({changed} of {total} words). Check it before accepting."
            ),
            Warning::Language { expected, got } => {
                write!(f, "The answer is in {got}, but your text is in {expected}.")
            }
            Warning::NotShorter => f.write_str("This is not shorter than your text."),
        }
    }
}

/// Serialised as `{"kind", "message"}` plus its fields (`changed` and `total`; `expected` and
/// `got`).
impl Serialize for Warning {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("kind", self.kind())?;
        map.serialize_entry("message", &self.to_string())?;
        match self {
            Warning::Guard { changed, total } => {
                map.serialize_entry("changed", changed)?;
                map.serialize_entry("total", total)?;
            }
            Warning::Language { expected, got } => {
                map.serialize_entry("expected", expected)?;
                map.serialize_entry("got", got)?;
            }
            Warning::NotShorter => {}
        }
        map.end()
    }
}

/// A rewrite's result. Serialised with its fields in this order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RewriteOutcome {
    /// The result, restored and cleaned, ready to replace the selection.
    pub text: String,
    /// From the selection to `text`.
    pub diff: Vec<DiffOp>,
    pub warnings: Vec<Warning>,
    /// Whether Enter accepts it: false after a guard or language warning.
    pub accept_default: bool,
    /// The result equals the selection: show [`NO_CHANGES`].
    pub unchanged: bool,
    /// The answer came from the cache, so nothing was sent.
    pub from_cache: bool,
    /// What publik API charged for the call, in micro-dollars; `None` from the cache and off
    /// publik API.
    pub charge_micros: Option<u64>,
}

fn thousands(n: &usize) -> String {
    group_thousands(*n)
}

/// Why a rewrite gave no result. `Display` is the message the user sees.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RewriteError {
    /// The provider call failed, or its answer could not be used (`InvalidAnswer`).
    #[error(transparent)]
    Ai(#[from] AiError),
    #[error("Select some text first.")]
    Empty,
    #[error("Select at most 1,500 words for a rewrite (this selection has {}).", thousands(.words))]
    TooLong { words: usize },
    #[error("Keep the instruction under 300 characters.")]
    InstructionTooLong,
    /// Custom with no instruction typed.
    #[error("Type an instruction for Custom first.")]
    InstructionMissing,
    #[error(
        "This text already contains a marker like [EMAIL_1], so redaction can't run safely. Turn off redaction in Settings → Privacy, or edit the text."
    )]
    RedactionCollision,
    #[error(
        "The AI answer dropped {marker}, so it was not used. Try again, or turn off redaction for this text."
    )]
    PlaceholderDropped { marker: String },
    #[error("The AI answer added a marker that was not in your text, so it was not used.")]
    PlaceholderInvented { marker: String },
}

impl RewriteError {
    /// The error's name: the variant's, or the `AiError`'s for `Ai`.
    pub fn kind(&self) -> &'static str {
        match self {
            RewriteError::Ai(e) => e.kind(),
            RewriteError::Empty => "Empty",
            RewriteError::TooLong { .. } => "TooLong",
            RewriteError::InstructionTooLong => "InstructionTooLong",
            RewriteError::InstructionMissing => "InstructionMissing",
            RewriteError::RedactionCollision => "RedactionCollision",
            RewriteError::PlaceholderDropped { .. } => "PlaceholderDropped",
            RewriteError::PlaceholderInvented { .. } => "PlaceholderInvented",
        }
    }

    /// Whether an answer arrived but failed validation (schema or markers): the case whose Try
    /// again sends a new idempotency key.
    pub fn is_invalid_answer(&self) -> bool {
        matches!(
            self,
            RewriteError::Ai(AiError::InvalidAnswer)
                | RewriteError::PlaceholderDropped { .. }
                | RewriteError::PlaceholderInvented { .. }
        )
    }

    /// `{"kind", "message"}` plus the variant's fields.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("an error serialises")
    }
}

/// Serialised as `{"kind", "message"}` plus `words` (`TooLong`) or `marker`; an `Ai` error as
/// the `AiError` itself.
impl Serialize for RewriteError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        if let RewriteError::Ai(e) = self {
            return e.serialize(serializer);
        }
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("kind", self.kind())?;
        map.serialize_entry("message", &self.to_string())?;
        match self {
            RewriteError::TooLong { words } => map.serialize_entry("words", words)?,
            RewriteError::PlaceholderDropped { marker }
            | RewriteError::PlaceholderInvented { marker } => {
                map.serialize_entry("marker", marker)?;
            }
            _ => {}
        }
        map.end()
    }
}

/// The provider a rewrite calls: on publik API and the fake provider, the model is the tier
/// `publik_tier` gives the action; the own key and a local server keep their own model.
pub fn provider_for(p: &ResolvedProvider, ctx: &TierContext, action: Action) -> ResolvedProvider {
    let mut provider = p.clone();
    if matches!(p.kind, ProviderKind::Publik | ProviderKind::Fake) {
        provider.chat_model = publik_tier(action.feature(), ctx).to_string();
    }
    provider
}

/// A request ready to send, and what its answer is checked against.
struct Prepared {
    provider: ResolvedProvider,
    call: ChatCall,
    redact: bool,
    entities: Vec<Entity>,
    /// Markers the answer must keep: those in the text as sent.
    required: Vec<String>,
    /// Markers the answer may hold: the text's and the instruction's.
    allowed: Vec<String>,
    unescape_text: bool,
    unescape_instruction: bool,
}

fn prepare(
    p: &ResolvedProvider,
    ctx: &TierContext,
    r: &RewriteRequest,
) -> Result<Prepared, RewriteError> {
    // The text is sent exactly as selected, its edges included; cleaning puts the original's
    // own whitespace back around the answer whatever the model does with it.
    let text = r.text.as_str();
    if text.trim().is_empty() {
        return Err(RewriteError::Empty);
    }
    let n = words(text);
    if n > MAX_WORDS {
        return Err(RewriteError::TooLong { words: n });
    }
    let instruction = if r.action.uses_instruction() {
        let typed = r.instruction.as_deref().map(str::trim).unwrap_or("");
        if typed.is_empty() {
            return Err(RewriteError::InstructionMissing);
        }
        if typed.chars().count() > MAX_INSTRUCTION_CHARS {
            return Err(RewriteError::InstructionTooLong);
        }
        Some(typed)
    } else {
        None
    };

    // Redaction: the instruction first, as it comes first in what is sent, then the text, with
    // one numbering so a value in both has one marker.
    let (sent_text, sent_instruction, entities) = if r.redact {
        let mut entities = Vec::new();
        let instruction = match instruction {
            Some(i) => Some(redact_into(i, &mut entities)?),
            None => None,
        };
        let text = redact_into(text, &mut entities)?;
        (text, instruction, entities)
    } else {
        (
            text.to_string(),
            instruction.map(str::to_string),
            Vec::new(),
        )
    };
    let required = markers_in(&sent_text);
    let mut allowed = required.clone();
    for m in markers_in(sent_instruction.as_deref().unwrap_or("")) {
        if !allowed.contains(&m) {
            allowed.push(m);
        }
    }

    let action = r.action;
    let prompt = action.prompt();
    let mut values: Vec<(&str, &str)> = vec![
        ("dialect_name", dialect_name(r.dialect)),
        ("text", sent_text.as_str()),
    ];
    if action.uses_style() {
        values.push(("style", r.style.sentence()));
    }
    if let Some(i) = &sent_instruction {
        values.push(("instruction", i.as_str()));
    }
    let rendered = prompts::render_prompt(prompt, &values).unwrap_or_else(|e| panic!("{e}"));

    let mut cache_input = json!({"text": sent_text, "dialect": r.dialect.as_str()});
    if action.uses_style() {
        cache_input["style"] = json!(r.style.as_str());
    }
    if action.has_variant() {
        cache_input["variant"] = json!(r.variant);
    }
    if let Some(i) = &sent_instruction {
        cache_input["instruction"] = json!(i);
    }

    let (text_escaped, text_instruction_escaped) = prompts::needs_escape(&sent_text);
    let (instruction_text_escaped, instruction_escaped) =
        prompts::needs_escape(sent_instruction.as_deref().unwrap_or(""));

    let call = ChatCall {
        feature: action.feature(),
        prompt_id: action.as_str(),
        prompt_version: prompt.version,
        messages: vec![
            Message::system(rendered.system),
            Message::user(rendered.user),
        ],
        max_tokens: prompt.max_tokens.for_words(words(&sent_text)),
        temperature: prompt.temperature,
        timeout: chat_timeout(p.kind),
        cache_input,
        cache: CachePolicy::Disk,
        validation_retry: r.validation_retry,
    };
    Ok(Prepared {
        provider: provider_for(p, ctx, action),
        call,
        redact: r.redact,
        entities,
        required,
        allowed,
        unescape_text: text_escaped || instruction_text_escaped,
        unescape_instruction: text_instruction_escaped || instruction_escaped,
    })
}

/// Redacts `text`, numbering after the `entities` found so far and reusing their markers for
/// repeated values; the new ones are added.
fn redact_into(text: &str, entities: &mut Vec<Entity>) -> Result<String, RewriteError> {
    let (own, found) = redact(text)?;
    if entities.is_empty() {
        *entities = found;
        return Ok(own);
    }
    // Renumber this part's markers after the ones already given.
    let mut renamed: Vec<(String, String)> = Vec::new();
    for e in found {
        let marker = match entities
            .iter()
            .find(|x| x.kind == e.kind && x.value == e.value)
        {
            Some(x) => x.marker.clone(),
            None => {
                let n = entities.iter().filter(|x| x.kind == e.kind).count() + 1;
                let marker = format!("[{}_{n}]", e.kind.label());
                entities.push(Entity {
                    marker: marker.clone(),
                    kind: e.kind,
                    value: e.value.clone(),
                });
                marker
            }
        };
        renamed.push((e.marker, marker));
    }
    Ok(MARKER
        .replace_all(&own, |caps: &regex::Captures<'_>| {
            renamed
                .iter()
                .find(|(from, _)| from == &caps[0])
                .map_or_else(|| caps[0].to_string(), |(_, to)| to.clone())
        })
        .into_owned())
}

/// The chat call a rewrite sends: messages, `max_tokens`, temperature, cache input and policy,
/// timeout and `validation_retry`. The model is [`provider_for`]'s.
pub fn build_request(
    p: &ResolvedProvider,
    ctx: &TierContext,
    r: &RewriteRequest,
) -> Result<ChatCall, RewriteError> {
    Ok(prepare(p, ctx, r)?.call)
}

/// The request body exactly as a rewrite sends it to `p`: `{"model", "messages", "max_tokens",
/// "temperature"}`.
pub fn request_body(
    p: &ResolvedProvider,
    ctx: &TierContext,
    r: &RewriteRequest,
) -> Result<Value, RewriteError> {
    let prepared = prepare(p, ctx, r)?;
    Ok(chat_body(&prepared.provider, &prepared.call))
}

/// The cache key K a rewrite's answer is stored under (64 lowercase hex). For a local server
/// with no model chosen, the client's K also names the model Ollama picked; this one leaves it
/// empty, which still tells inputs apart.
pub fn cache_key(
    p: &ResolvedProvider,
    ctx: &TierContext,
    r: &RewriteRequest,
) -> Result<String, RewriteError> {
    let prepared = prepare(p, ctx, r)?;
    Ok(AiClient::cache_key(&prepared.provider, &prepared.call))
}

/// An answer that passed validation.
struct Answer {
    text: String,
    changes_language: bool,
}

/// Validation of one answer's content (module comment, step 5).
fn check_answer(
    content: &str,
    r: &RewriteRequest,
    prepared: &Prepared,
) -> Result<Answer, RewriteError> {
    let schema = &r.action.prompt().schema;
    let value = parse::parse_answer(content, schema)?;
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .ok_or(AiError::InvalidAnswer)?;
    let changes_language = value
        .get("changes_language")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let text = prompts::unescape(text, prepared.unescape_text, prepared.unescape_instruction);
    let restored = if prepared.redact {
        let found = markers_in(&text);
        if let Some(missing) = prepared.required.iter().find(|m| !found.contains(m)) {
            return Err(RewriteError::PlaceholderDropped {
                marker: missing.clone(),
            });
        }
        if let Some(extra) = found.iter().find(|m| !prepared.allowed.contains(m)) {
            return Err(RewriteError::PlaceholderInvented {
                marker: extra.clone(),
            });
        }
        restore(&text, &prepared.entities)
    } else {
        text
    };
    let cleaned = clean::clean(&restored, &r.text);
    if cleaned.trim().is_empty() {
        return Err(AiError::InvalidAnswer.into());
    }
    Ok(Answer {
        text: cleaned,
        changes_language,
    })
}

/// The outcome of a valid answer (module comment, step 6).
fn finish(r: &RewriteRequest, answer: Answer, reply: &ChatReply) -> RewriteOutcome {
    let unchanged = answer.text == r.text;
    let mut warnings = Vec::new();
    if !unchanged {
        let keeps_language = !(r.action == Action::Custom && answer.changes_language);
        if keeps_language
            && let Some((expected, got)) = crate::lang::changed_language(&r.text, &answer.text)
        {
            warnings.push(Warning::Language {
                expected: expected.to_string(),
                got: got.to_string(),
            });
        }
        if r.action.guarded()
            && let Some(w) = guard::guard(&r.text, &answer.text)
        {
            warnings.push(w);
        }
        if r.action.must_be_shorter()
            && let Some(w) = guard::not_shorter(&r.text, &answer.text)
        {
            warnings.push(w);
        }
    }
    RewriteOutcome {
        diff: word_diff(&r.text, &answer.text),
        accept_default: !warnings.iter().any(Warning::blocks_default),
        warnings,
        unchanged,
        from_cache: reply.from_cache,
        charge_micros: reply.charge_micros,
        text: answer.text,
    }
}

/// Runs one rewrite through `c` on `p`. Refusals (`Empty`, `TooLong`, `InstructionTooLong`,
/// `InstructionMissing`, `RedactionCollision`) send nothing.
pub async fn rewrite(
    c: &AiClient,
    p: &ResolvedProvider,
    ctx: &TierContext,
    r: RewriteRequest,
) -> Result<RewriteOutcome, RewriteError> {
    let result = run(c, p, ctx, &r).await;
    match &result {
        Ok(o) => tracing::info!(
            action = r.action.as_str(),
            from_cache = o.from_cache,
            unchanged = o.unchanged,
            warnings = o.warnings.len(),
            "rewrite done"
        ),
        Err(e) => tracing::info!(
            action = r.action.as_str(),
            error = e.kind(),
            "rewrite stopped"
        ),
    }
    result
}

async fn run(
    c: &AiClient,
    p: &ResolvedProvider,
    ctx: &TierContext,
    r: &RewriteRequest,
) -> Result<RewriteOutcome, RewriteError> {
    let prepared = prepare(p, ctx, r)?;
    let (reply, answer) = c
        .chat_validated(&prepared.provider, prepared.call.clone(), |content| {
            check_answer(content, r, &prepared)
        })
        .await?;
    Ok(finish(r, answer, &reply))
}

/// The count of failed answers per cache key, for as long as a popup keeps it: the "session"
/// whose Try again after an invalid answer gets a new idempotency key.
#[derive(Debug, Default)]
pub struct ValidationRetries {
    counts: Mutex<HashMap<String, u32>>,
}

impl ValidationRetries {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many answers for cache key `key` failed validation so far.
    pub fn count(&self, key: &str) -> u32 {
        self.lock().get(key).copied().unwrap_or(0)
    }

    /// Counts a result for `key`: an answer that failed validation adds one; a result resets the
    /// count; any other error leaves it, so its Try again sends the same key again.
    pub fn record(&self, key: &str, result: &Result<RewriteOutcome, RewriteError>) {
        let mut counts = self.lock();
        match result {
            Ok(_) => {
                counts.remove(key);
            }
            Err(e) if e.is_invalid_answer() => *counts.entry(key.to_string()).or_insert(0) += 1,
            Err(_) => {}
        }
    }

    /// Runs `r` with `validation_retry` set from the count for its cache key, and counts the
    /// result.
    pub async fn rewrite(
        &self,
        c: &AiClient,
        p: &ResolvedProvider,
        ctx: &TierContext,
        mut r: RewriteRequest,
    ) -> Result<RewriteOutcome, RewriteError> {
        let key = cache_key(p, ctx, &r)?;
        r.validation_retry = self.count(&key);
        let result = rewrite(c, p, ctx, r).await;
        self.record(&key, &result);
        result
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, u32>> {
        self.counts.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests;
