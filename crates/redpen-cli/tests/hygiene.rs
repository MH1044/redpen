//! Source-hygiene test (docs/TESTING.md §5).
//!
//! Fails the build when a file that could be published contains something that would identify a
//! person or a machine, or something shaped like a secret. It runs in the normal test suite, so in
//! `scripts/verify` and in CI. It names no person and no address.
//!
//! It lives in this crate's `tests/` directory because a root `tests/` folder is never compiled in
//! a Cargo virtual workspace; it walks up from `CARGO_MANIFEST_DIR` to the repository root.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::bytes::Regex;

/// Text is decided by extension, never by sniffing, so a new binary fixture cannot opt itself in.
const TEXT_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "toml", "json", "jsonc",
    "yml", "yaml", "md", "html", "css", "txt", "sh", "ps1", "xml", "svg", "csv", "tsv", "lock",
];

/// Exempt from the email rule only: upstream metadata the project neither writes nor controls.
const EMAIL_EXEMPT: &[&str] = &[
    "package-lock.json",
    "Cargo.lock",
    "uv.lock",
    "THIRD_PARTY.md",
];

/// The one file allowed to hold a Publik app token (public by design, docs/PUBLIK-CONTRACT.md §2).
const APP_TOKEN_FILE: &str = "app/src-tauri/publik-app-token.txt";

/// About half the file count when this was last set, so an empty walk cannot pass vacuously.
const MIN_FILES: usize = 41;

/// Skipped by the fallback walk (git normally decides), everywhere in the tree.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    "release",
    ".venv",
    "__pycache__",
];

struct TextFile {
    /// Relative to the repository root, with forward slashes.
    path: String,
    bytes: Vec<u8>,
}

/// The repository root: the first ancestor holding `.git` or the workspace Cargo.toml.
fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join(".git").exists() {
            return dir;
        }
        if let Ok(text) = fs::read_to_string(dir.join("Cargo.toml"))
            && text.contains("[workspace]")
        {
            return dir;
        }
        assert!(dir.pop(), "no repository root above the crate");
    }
}

fn is_text(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if name.starts_with("LICENSE") || name.starts_with("COPYING") {
        return true;
    }
    match name.rsplit_once('.') {
        Some((_, ext)) => TEXT_EXTENSIONS.contains(&ext),
        None => false,
    }
}

/// The files git would publish: tracked plus untracked-but-not-ignored (`-z`, so non-ASCII names
/// are not quoted), skipping paths that no longer exist on disk.
fn git_listed(root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|b| *b == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf8_lossy(entry).into_owned())
            .collect(),
    )
}

/// Without git: walk the tree honouring .gitignore, skipping the usual build and dependency
/// directories, and the root `models/` and `eval-data/` folders except `models/registry.toml`.
fn walked(root: &Path) -> Vec<String> {
    let root_owned = root.to_path_buf();
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(move |entry| {
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_dir && SKIP_DIRS.contains(&name.as_str()) {
                return false;
            }
            let rel = entry
                .path()
                .strip_prefix(&root_owned)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if rel.starts_with("eval-data/") || rel == "eval-data" {
                return false;
            }
            if rel.starts_with("models/") || rel == "models" {
                return rel == "models" || rel == "models/registry.toml";
            }
            true
        })
        .build()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .collect()
}

fn text_files() -> Vec<TextFile> {
    let root = repo_root();
    let listed = git_listed(&root).unwrap_or_else(|| walked(&root));
    let mut files: Vec<TextFile> = listed
        .into_iter()
        .filter(|rel| is_text(rel))
        .filter_map(|rel| {
            let bytes = fs::read(root.join(&rel)).ok()?;
            Some(TextFile { path: rel, bytes })
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// Every `path:line` where `pattern` matches, over every file that `include` keeps.
fn hits(pattern: &Regex, include: impl Fn(&TextFile) -> bool) -> Vec<String> {
    let mut found = Vec::new();
    for file in text_files().iter().filter(|f| include(f)) {
        for (i, line) in file.bytes.split(|b| *b == b'\n').enumerate() {
            if pattern.is_match(line) {
                found.push(format!("{}:{}", file.path, i + 1));
            }
        }
    }
    found
}

// Test 1: the walk found the tree.
#[test]
fn hygiene_walk_finds_the_tree() {
    let count = text_files().len();
    assert!(
        count > MIN_FILES,
        "only {count} text files were found; the walk is not seeing the repository"
    );
}

// Test 2: no control characters other than tab, LF and CR. Tooling that edits a file through a
// JSON string can decode an escape into the character it denotes; the file still compiles, but
// it has stopped being text.
#[test]
fn hygiene_no_control_characters() {
    let pattern = Regex::new(r"[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]").unwrap();
    let found = hits(&pattern, |_| true);
    assert!(
        found.is_empty(),
        "control characters in:\n{}\nwrite the character as an escape sequence",
        found.join("\n")
    );
}

// Test 3: no home-directory paths: a drive letter, a colon, one or more separators, the Windows
// users folder and another separator run, or the two Unix home folders, followed by a name. The
// `+` after each separator catches the doubled backslashes of a JSON-escaped Windows path.
#[test]
fn hygiene_no_home_directory_paths() {
    let pattern =
        Regex::new(r"(?:[A-Za-z]:[\\/]+Users[\\/]+|/home/|/Users/)[A-Za-z0-9._-]+").unwrap();
    let found = hits(&pattern, |_| true);
    assert!(
        found.is_empty(),
        "home-directory paths in:\n{}\nuse a path relative to the repository root, or an environment variable",
        found.join("\n")
    );
}

// Test 4: no email addresses except the noreply identities and the documentation domains.
#[test]
fn hygiene_no_email_addresses() {
    let email = Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap();
    let allowed = [
        Regex::new(r"^\d+\+[A-Za-z0-9-]+@users\.noreply\.github\.com$").unwrap(),
        Regex::new(r"^noreply@(?:anthropic|github)\.com$").unwrap(),
        Regex::new(r"@(?:example|invalid|localhost|test)(?:\.[A-Za-z]{2,})?$").unwrap(),
        Regex::new(r"\.(?:example|invalid|test)$").unwrap(),
    ];
    let mut found = Vec::new();
    for file in text_files() {
        let name = file.path.rsplit('/').next().unwrap_or(&file.path);
        if EMAIL_EXEMPT.contains(&name) {
            continue;
        }
        for (i, line) in file.bytes.split(|b| *b == b'\n').enumerate() {
            for m in email.find_iter(line) {
                if !allowed.iter().any(|a| a.is_match(m.as_bytes())) {
                    found.push(format!("{}:{}", file.path, i + 1));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "email addresses in:\n{}\na real address in a public repository cannot be taken back",
        found.join("\n")
    );
}

// Test 5: no secret-shaped strings. Test fakes stay short on purpose. The one allowed token is
// the Publik app token inside app/src-tauri/publik-app-token.txt, public by design.
#[test]
fn hygiene_no_secret_shaped_strings() {
    let pattern = Regex::new(concat!(
        r"pk_(?:live|test)_[A-Za-z0-9]{16,}",
        r"|pbt_[A-Za-z0-9]{16,}",
        r"|sk-[A-Za-z0-9_-]{20,}",
        r"|AIza[0-9A-Za-z_-]{35}",
        r"|gh[pousr]_[A-Za-z0-9]{30,}",
        r"|github_pat_[A-Za-z0-9_]{30,}",
        r"|xox[baprs]-[A-Za-z0-9-]{10,}",
        r"|-----BEGIN (?:RSA |EC |OPENSSH |)PRIVATE KEY-----",
    ))
    .unwrap();
    let app_token = Regex::new(r"\bpat_[A-Za-z0-9]+_[A-Za-z0-9_-]{16,}").unwrap();
    let mut found = hits(&pattern, |_| true);
    found.extend(hits(&app_token, |f| f.path != APP_TOKEN_FILE));
    found.sort();
    assert!(
        found.is_empty(),
        "secret-shaped strings in:\n{}\nremove the key and rotate it",
        found.join("\n")
    );
}
