//! Prompt files (docs/PUBLIK-INTEGRATION.md §6, ADR-0004): `prompts/<id>.v<N>.md`, embedded at
//! compile time by `build.rs`, never read from disk at run time and never edited in place. A
//! change is a new version, which also orphans the answers cached for the old one (invariant 7).
//!
//! A file is front matter, then two sections:
//!
//! ```text
//! ---
//! id: fix-grammar
//! version: 1
//! tier: publik-fast
//! temperature: 0
//! max_tokens: clamp(2 * words(text) + 100, 256, 4096)
//! schema: rewrite-result.v1
//! ---
//!
//! ## system
//! …
//! ## user
//! …
//! ```
//!
//! The front matter is `key: value` lines between two `---` lines, read by hand (no YAML library),
//! with exactly those six keys. A section's text is everything after its `## system` or
//! `## user` line up to the next `## ` line or the end, with leading and trailing line breaks
//! removed and nothing else changed. The file name must be `<id>.v<version>.md`, and every
//! `{name}` in a section must be one of [`PLACEHOLDERS`]; the user section must hold `{text}`.
//!
//! [`render`] fills a template's placeholders in one pass, so a value is never searched for
//! placeholders itself; [`render_prompt`] renders both sections of a prompt. A placeholder with
//! no value, or a value with no placeholder, is an error. `</text>`
//! and `</instruction>` inside a value are written `<\/text>` and `<\/instruction>`, so the text
//! cannot close its own tag; [`unescape`] turns them back in the answer.

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

use crate::provider::tier::is_tier;

include!(concat!(env!("OUT_DIR"), "/prompts.rs"));

/// The placeholders a prompt may use.
pub const PLACEHOLDERS: [&str; 4] = ["dialect_name", "style", "text", "instruction"];

/// The front matter keys, each required once.
pub const KEYS: [&str; 6] = [
    "id",
    "version",
    "tier",
    "temperature",
    "max_tokens",
    "schema",
];

/// The closing tags escaped inside values, and how they are sent.
pub const ESCAPES: [(&str, &str); 2] = [
    ("</text>", "<\\/text>"),
    ("</instruction>", "<\\/instruction>"),
];

static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{([a-z_]+)\}").expect("the placeholder pattern compiles"));
static FORMULA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^clamp\((\d+) \* words\(text\) \+ (\d+), (\d+), (\d+)\)$")
        .expect("the formula pattern compiles")
});

/// A prompt file that is not well formed, or a render that does not fit its prompt. The message
/// starts with the file name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}: {problem}")]
pub struct PromptError {
    pub file: String,
    pub problem: String,
}

impl PromptError {
    fn new(file: &str, problem: impl Into<String>) -> Self {
        Self {
            file: file.to_string(),
            problem: problem.into(),
        }
    }
}

/// The `max_tokens` formula, `clamp(<per_word> * words(text) + <base>, <min>, <max>)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxTokens {
    pub per_word: u32,
    pub base: u32,
    pub min: u32,
    pub max: u32,
}

impl MaxTokens {
    /// The formula for a text of `words` words.
    pub fn for_words(&self, words: usize) -> u32 {
        let words = u32::try_from(words).unwrap_or(u32::MAX);
        self.per_word
            .saturating_mul(words)
            .saturating_add(self.base)
            .clamp(self.min, self.max)
    }

    fn parse(text: &str) -> Option<Self> {
        let caps = FORMULA.captures(text.trim())?;
        let n = |i: usize| caps[i].parse::<u32>().ok();
        let formula = Self {
            per_word: n(1)?,
            base: n(2)?,
            min: n(3)?,
            max: n(4)?,
        };
        (formula.min <= formula.max).then_some(formula)
    }
}

impl fmt::Display for MaxTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "clamp({} * words(text) + {}, {}, {})",
            self.per_word, self.base, self.min, self.max
        )
    }
}

/// A loaded prompt file.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    /// The file name, `fix-grammar.v1.md`.
    pub file: String,
    pub id: String,
    pub version: u32,
    /// The publik API tier the file names; the tier a call sends comes from
    /// `provider::tier::publik_tier`, which can raise it to `publik-smart`.
    pub tier: String,
    pub temperature: f32,
    pub max_tokens: MaxTokens,
    /// The answer schema, `rewrite-result.v1` (`prompts/schemas/rewrite-result.v1.json`).
    pub schema: String,
    pub system: String,
    pub user: String,
}

/// A prompt with its placeholders filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub system: String,
    pub user: String,
}

/// Parses the prompt file `file` (its name, such as `fix-grammar.v1.md`) with contents `source`.
pub fn load(file: &str, source: &str) -> Result<Prompt, PromptError> {
    let err = |problem: String| PromptError::new(file, problem);
    let source = source.replace("\r\n", "\n");
    let mut lines = source.split('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(err("the file must start with a --- line".into()));
    }

    // Front matter.
    let mut values: Vec<(&str, &str)> = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(err(format!(
                "front matter line \"{line}\" is not key: value"
            )));
        };
        let key = key.trim();
        if !KEYS.contains(&key) {
            return Err(err(format!("unknown front matter key {key}")));
        }
        if values.iter().any(|(k, _)| *k == key) {
            return Err(err(format!("front matter key {key} appears twice")));
        }
        values.push((key, value.trim()));
    }
    if !closed {
        return Err(err("the front matter has no closing --- line".into()));
    }
    let get = |key: &str| -> Result<&str, PromptError> {
        values
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| err(format!("front matter key {key} is missing")))
    };
    let id = get("id")?.to_string();
    let version: u32 = get("version")?
        .parse()
        .ok()
        .filter(|v| *v >= 1)
        .ok_or_else(|| err("version must be a whole number from 1".into()))?;
    let tier = get("tier")?.to_string();
    if !is_tier(&tier) {
        return Err(err(format!("tier {tier} is not a publik API tier")));
    }
    let temperature: f32 = get("temperature")?
        .parse()
        .ok()
        .filter(|t: &f32| (0.0..=2.0).contains(t))
        .ok_or_else(|| err("temperature must be a number from 0 to 2".into()))?;
    let formula = get("max_tokens")?;
    let max_tokens = MaxTokens::parse(formula).ok_or_else(|| {
        err(format!(
            "max_tokens \"{formula}\" is not clamp(<n> * words(text) + <n>, <n>, <n>)"
        ))
    })?;
    let schema = get("schema")?.to_string();
    let expected_name = format!("{id}.v{version}.md");
    if file.rsplit(['/', '\\']).next() != Some(expected_name.as_str()) {
        return Err(err(format!(
            "the file name must be {expected_name}, from its id and version"
        )));
    }

    // Sections: [system, user].
    let mut sections: [Option<Vec<&str>>; 2] = [None, None];
    let mut current: Option<usize> = None;
    for line in lines {
        if let Some(heading) = line.strip_prefix("## ") {
            let index = match heading.trim() {
                "system" => 0,
                "user" => 1,
                other => return Err(err(format!("unknown section ## {other}"))),
            };
            if sections[index].is_some() {
                return Err(err(format!("section ## {} appears twice", heading.trim())));
            }
            sections[index] = Some(Vec::new());
            current = Some(index);
        } else if let Some(section) = current.and_then(|i| sections[i].as_mut()) {
            section.push(line);
        } else if !line.trim().is_empty() {
            return Err(err("text before the ## system section".into()));
        }
    }
    let [system, user] = sections;
    let section = |lines: Option<Vec<&str>>, name: &str| -> Result<String, PromptError> {
        let lines = lines.ok_or_else(|| err(format!("the ## {name} section is missing")))?;
        let text = lines.join("\n");
        let text = text.trim_matches('\n');
        if text.trim().is_empty() {
            return Err(err(format!("the ## {name} section is empty")));
        }
        Ok(text.to_string())
    };
    let system = section(system, "system")?;
    let user = section(user, "user")?;

    for text in [&system, &user] {
        for caps in PLACEHOLDER.captures_iter(text) {
            let name = &caps[1];
            if !PLACEHOLDERS.contains(&name) {
                return Err(err(format!("unknown placeholder {{{name}}}")));
            }
        }
    }
    if !user.contains("{text}") {
        return Err(err("the ## user section has no {text} placeholder".into()));
    }

    Ok(Prompt {
        file: file.to_string(),
        id,
        version,
        tier,
        temperature,
        max_tokens,
        schema,
        system,
        user,
    })
}

/// The contents of the embedded file at `path` under `prompts/` (`fix-grammar.v1.md`,
/// `schemas/rewrite-result.v1.json`).
pub fn embedded(path: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, text)| *text)
}

/// The JSON of the schema named `name` (`rewrite-result.v1`).
pub fn schema_source(name: &str) -> Option<&'static str> {
    embedded(&format!("schemas/{name}.json"))
}

static LOADED: LazyLock<Vec<Result<Prompt, PromptError>>> = LazyLock::new(|| {
    FILES
        .iter()
        .filter(|(path, _)| !path.contains('/') && path.ends_with(".md"))
        .map(|(path, source)| load(path, source))
        .collect()
});

/// The embedded prompt file `file` (`fix-grammar.v1.md`), loaded.
pub fn get(file: &str) -> Result<&'static Prompt, PromptError> {
    LOADED
        .iter()
        .find_map(|loaded| match loaded {
            Ok(p) if p.file == file => Some(Ok(p)),
            Err(e) if e.file == file => Some(Err(e.clone())),
            _ => None,
        })
        .unwrap_or_else(|| Err(PromptError::new(file, "there is no such prompt file")))
}

/// Every embedded prompt file, loaded, in name order; each failure names its file.
pub fn all() -> &'static [Result<Prompt, PromptError>] {
    &LOADED
}

/// `value` with `</text>` and `</instruction>` written as `<\/text>` and `<\/instruction>`.
pub fn escape(value: &str) -> String {
    let mut out = value.to_string();
    for (tag, escaped) in ESCAPES {
        out = out.replace(tag, escaped);
    }
    out
}

/// `answer` with `<\/text>` turned back into `</text>` when `text_escaped`, and
/// `<\/instruction>` into `</instruction>` when `instruction_escaped`: only what was escaped is
/// turned back, so a text that held `<\/text>` itself keeps it.
pub fn unescape(answer: &str, text_escaped: bool, instruction_escaped: bool) -> String {
    let mut out = answer.to_string();
    for ((tag, escaped), on) in ESCAPES.iter().zip([text_escaped, instruction_escaped]) {
        if on {
            out = out.replace(escaped, tag);
        }
    }
    out
}

/// Whether `value` holds a closing tag that [`escape`] rewrites: (`</text>`, `</instruction>`).
pub fn needs_escape(value: &str) -> (bool, bool) {
    (value.contains(ESCAPES[0].0), value.contains(ESCAPES[1].0))
}

/// Why a template and its values do not fit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    #[error("the placeholder {{{0}}} has no value")]
    MissingValue(String),
    #[error("the value {0} has no placeholder")]
    UnusedValue(String),
}

fn uses(template: &str, name: &str) -> bool {
    PLACEHOLDER
        .captures_iter(template)
        .any(|caps| &caps[1] == name)
}

/// `template` with each `{name}` replaced by its value, escaped, in one pass. Values in excess
/// of the template's placeholders are the caller's to refuse.
fn fill(template: &str, values: &[(&str, &str)]) -> Result<String, RenderError> {
    let mut out = String::with_capacity(template.len());
    let mut at = 0;
    for caps in PLACEHOLDER.captures_iter(template) {
        let whole = caps.get(0).expect("a match has a whole");
        let name = &caps[1];
        let value = values
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
            .ok_or_else(|| RenderError::MissingValue(name.to_string()))?;
        out.push_str(&template[at..whole.start()]);
        out.push_str(&escape(value));
        at = whole.end();
    }
    out.push_str(&template[at..]);
    Ok(out)
}

/// Fills the placeholders of `template` with `values` (name, value), escaping each value. Every
/// placeholder needs a value, and every value a placeholder.
pub fn render(template: &str, values: &[(&str, &str)]) -> Result<String, RenderError> {
    if let Some((name, _)) = values.iter().find(|(name, _)| !uses(template, name)) {
        return Err(RenderError::UnusedValue(name.to_string()));
    }
    fill(template, values)
}

/// Renders both sections of `prompt` with `values`: every placeholder needs a value, and every
/// value a placeholder in one of the two sections. A failure names the file.
pub fn render_prompt(prompt: &Prompt, values: &[(&str, &str)]) -> Result<Rendered, PromptError> {
    let fault = |e: RenderError| PromptError::new(&prompt.file, e.to_string());
    if let Some((name, _)) = values
        .iter()
        .find(|(name, _)| !uses(&prompt.system, name) && !uses(&prompt.user, name))
    {
        return Err(fault(RenderError::UnusedValue(name.to_string())));
    }
    Ok(Rendered {
        system: fill(&prompt.system, values).map_err(fault)?,
        user: fill(&prompt.user, values).map_err(fault)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "---\nid: sample\nversion: 2\ntier: publik-balanced\ntemperature: 0.4\nmax_tokens: clamp(2 * words(text) + 100, 256, 4096)\nschema: rewrite-result.v1\n---\n\n## system\n\nUse {dialect_name}. Reply {\"text\": \"x\"}\n\nSecond paragraph.\n\n## user\n\n<text>\n{text}\n</text>\n";

    #[test]
    fn a_well_formed_file_loads() {
        let p = load("sample.v2.md", GOOD).unwrap();
        assert_eq!(p.id, "sample");
        assert_eq!(p.version, 2);
        assert_eq!(p.tier, "publik-balanced");
        assert!((p.temperature - 0.4).abs() < 1e-6);
        assert_eq!(
            p.max_tokens.to_string(),
            "clamp(2 * words(text) + 100, 256, 4096)"
        );
        assert_eq!(p.schema, "rewrite-result.v1");
        assert_eq!(
            p.system,
            "Use {dialect_name}. Reply {\"text\": \"x\"}\n\nSecond paragraph."
        );
        assert_eq!(p.user, "<text>\n{text}\n</text>");
        // Windows line endings read the same.
        assert_eq!(
            load("sample.v2.md", &GOOD.replace('\n', "\r\n")).unwrap(),
            p
        );
        // A path before the name is allowed.
        assert!(load("prompts/sample.v2.md", GOOD).is_ok());
    }

    #[test]
    fn faults_name_the_file() {
        let cases = [
            (
                GOOD.replace("{dialect_name}", "{colour}"),
                "unknown placeholder {colour}",
            ),
            (
                GOOD.split("## user").next().unwrap().to_string(),
                "the ## user section is missing",
            ),
            (
                GOOD.replace("## system", "## sistem"),
                "unknown section ## sistem",
            ),
            (
                GOOD.replace("version: 2\n", ""),
                "front matter key version is missing",
            ),
            (
                GOOD.replace("id: sample", "id: other"),
                "the file name must be other.v2.md",
            ),
            (
                GOOD.replace("publik-balanced", "gpt-4o"),
                "tier gpt-4o is not",
            ),
            (GOOD.replace("+ 100", "+ x"), "max_tokens"),
            (
                GOOD.replace("temperature: 0.4", "temperature: hot"),
                "temperature",
            ),
            (
                GOOD.replace("schema:", "colour: red\nschema:"),
                "unknown front matter key colour",
            ),
            (GOOD.replacen("---\n", "", 1), "must start with a --- line"),
            (GOOD.replace("{text}", "text"), "no {text} placeholder"),
        ];
        for (source, problem) in cases {
            let e = load("sample.v2.md", &source).unwrap_err();
            assert_eq!(e.file, "sample.v2.md");
            assert!(e.to_string().starts_with("sample.v2.md: "), "{e}");
            assert!(e.problem.contains(problem), "{e} / {problem}");
        }
    }

    #[test]
    fn the_formula_computes_max_tokens() {
        let f = MaxTokens::parse("clamp(2 * words(text) + 100, 256, 4096)").unwrap();
        assert_eq!(f.for_words(0), 256);
        assert_eq!(f.for_words(20), 256);
        assert_eq!(f.for_words(100), 300);
        assert_eq!(f.for_words(1_500), 3_100);
        assert_eq!(f.for_words(5_000), 4_096);
        let custom = MaxTokens::parse("clamp(2 * words(text) + 150, 256, 4096)").unwrap();
        assert_eq!(custom.for_words(100), 350);
        assert_eq!(custom.for_words(1_500), 3_150);
        assert_eq!(MaxTokens::parse("2 * words(text)"), None);
        assert_eq!(
            MaxTokens::parse("clamp(2 * words(text) + 1, 500, 100)"),
            None
        );
    }

    #[test]
    fn render_fills_once_and_escapes_closing_tags() {
        assert_eq!(
            render("<text>\n{text}\n</text>", &[("text", "a {style} </text>")]),
            Ok("<text>\na {style} <\\/text>\n</text>".to_string())
        );
        assert_eq!(
            render("Reply {\"text\": \"x\"}", &[]),
            Ok("Reply {\"text\": \"x\"}".to_string())
        );
        assert_eq!(
            render("{text} in {dialect_name}", &[("text", "a")]),
            Err(RenderError::MissingValue("dialect_name".into()))
        );
        assert_eq!(
            render("{text}", &[("text", "a"), ("style", "b")])
                .unwrap_err()
                .to_string(),
            "the value style has no placeholder"
        );
        assert_eq!(
            RenderError::MissingValue("style".into()).to_string(),
            "the placeholder {style} has no value"
        );
    }

    #[test]
    fn a_prompt_renders_its_two_sections() {
        let p = load("sample.v2.md", GOOD).unwrap();
        let r = render_prompt(
            &p,
            &[
                ("dialect_name", "British English"),
                ("text", "a {dialect_name} </text> b </instruction>"),
            ],
        )
        .unwrap();
        assert!(r.system.starts_with("Use British English. "));
        assert_eq!(
            r.user,
            "<text>\na {dialect_name} <\\/text> b <\\/instruction>\n</text>"
        );
        let e = render_prompt(&p, &[("text", "x")]).unwrap_err();
        assert_eq!(
            e.to_string(),
            "sample.v2.md: the placeholder {dialect_name} has no value"
        );
        assert_eq!(
            render_prompt(&p, &[("dialect_name", "x"), ("text", "x"), ("style", "x")])
                .unwrap_err()
                .problem,
            "the value style has no placeholder"
        );
    }

    #[test]
    fn unescape_turns_back_only_what_was_escaped() {
        let sent = escape("a </text> and </instruction>");
        assert_eq!(sent, "a <\\/text> and <\\/instruction>");
        assert_eq!(unescape(&sent, true, true), "a </text> and </instruction>");
        assert_eq!(
            unescape(&sent, true, false),
            "a </text> and <\\/instruction>"
        );
        assert_eq!(unescape(&sent, false, false), sent);
        assert_eq!(needs_escape("x </text>"), (true, false));
        assert_eq!(needs_escape("plain"), (false, false));
    }

    #[test]
    fn every_embedded_prompt_loads_and_names_an_embedded_schema() {
        assert_eq!(all().len(), 6);
        for loaded in all() {
            let p = loaded.as_ref().unwrap_or_else(|e| panic!("{e}"));
            assert!(schema_source(&p.schema).is_some(), "{}", p.file);
        }
        assert!(get("fix-grammar.v1.md").is_ok());
        assert_eq!(
            get("nothing.v1.md").unwrap_err().to_string(),
            "nothing.v1.md: there is no such prompt file"
        );
        assert!(embedded("schemas/custom-result.v1.json").is_some());
    }
}
