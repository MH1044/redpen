//! The copy rule (docs/PUBLIK-CONTRACT.md §9, docs/TESTING.md §5): Redpen talks in dollars. The
//! interface (`app/src`), `README.md` and the documentation (`docs/`) never use the phrases the
//! contract forbids. `docs/PUBLIK-CONTRACT.md` quotes them and is exempt, and so is this file,
//! which is outside the folders it scans.
//!
//! It walks up from `CARGO_MANIFEST_DIR` to the repository root, as the hygiene test does.

use std::fs;
use std::path::{Path, PathBuf};

/// The phrases contract §9 forbids, matched without regard to case.
const FORBIDDEN: [&str; 5] = [
    "publik credits",
    "in credits",
    "Gemini credit",
    "ChatGPT credit",
    "OpenAI API",
];

/// The one document exempt from the rule: the shared contract, which quotes the phrases.
const EXEMPT: &str = "docs/PUBLIK-CONTRACT.md";

/// Skipped wherever they appear: generated or installed, never written by hand.
const SKIP_DIRS: [&str; 3] = ["node_modules", "dist", "target"];

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

/// Every forbidden phrase under `root`'s `app/src`, `README.md` and `docs/`, as
/// `path:line: "phrase"`, paths relative to `root` with forward slashes.
fn violations(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect(&root.join("app").join("src"), &mut files);
    collect(&root.join("docs"), &mut files);
    let readme = root.join("README.md");
    if readme.is_file() {
        files.push(readme);
    }
    let mut found = Vec::new();
    for file in files {
        let shown = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        if shown == EXEMPT {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            for phrase in FORBIDDEN {
                if lower.contains(&phrase.to_lowercase()) {
                    found.push(format!("{shown}:{}: {phrase:?}", n + 1));
                }
            }
        }
    }
    found.sort();
    found
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if !SKIP_DIRS.iter().any(|s| name == *s) {
                collect(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

#[test]
fn the_interface_and_the_documentation_talk_in_dollars() {
    let root = repo_root();
    assert!(root.join(EXEMPT).is_file(), "the contract is in docs/");
    let found = violations(&root);
    assert!(
        found.is_empty(),
        "phrases the publik contract forbids (§9):\n{}\ntalk in dollars: write the amount, for example \"$0.05 of free use\"",
        found.join("\n")
    );
}

#[test]
fn a_planted_ui_string_fails_and_the_contract_is_exempt() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("app").join("src").join("windows")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(
        root.join("app")
            .join("src")
            .join("windows")
            .join("Card.tsx"),
        "export const label = 'Buy more publik credits'\n",
    )
    .unwrap();
    fs::write(root.join("README.md"), "Costs are paid in credits.\n").unwrap();
    fs::write(
        root.join(EXEMPT),
        "A test fails on \"publik credits\" or \"OpenAI API\".\n",
    )
    .unwrap();
    fs::write(
        root.join("docs").join("HANDBOOK.md"),
        "About $0.002 a rewrite.\n",
    )
    .unwrap();
    assert_eq!(
        violations(root),
        [
            "README.md:1: \"in credits\"",
            "app/src/windows/Card.tsx:1: \"publik credits\"",
        ]
    );
    // Any letter case is caught.
    fs::write(
        root.join("docs").join("HANDBOOK.md"),
        "Uses the openai api.\n",
    )
    .unwrap();
    assert!(
        violations(root)
            .iter()
            .any(|v| v.starts_with("docs/HANDBOOK.md:1:"))
    );
}
