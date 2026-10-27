//! The golden harness (docs/TESTING.md §1): every `tests/cases/NN-slug/<case>/cmd.json` is one
//! test that runs the `redpen` binary with the case directory as working directory, checks the
//! exit code, and compares its standard output, parsed as JSON, with the case's `expected.json`.
//!
//! `cmd.json`: `{"args": ["rewrite", "--action", "fix-grammar", "--provider", "fake",
//! "input.txt"], "exit": 0}` (`exit` defaults to 0). Paths in `args` are relative to the case
//! directory, which also holds the case's `input.*` and `recording.json`.
//!
//! `GOLDEN_FILTER=<prefix>` runs only the cases whose `NN-slug/case` id starts with the prefix.
//! `UPDATE_GOLDEN=1` writes an `expected.json` that is missing, or rewrites the one case named
//! exactly by `GOLDEN_FILTER`; it never turns a red test green by itself, because a golden file
//! is the specification, not a cache: read the diff line by line.
//!
//! One module: `GOLDEN_FILTER=04-rewrite-engine cargo test -p redpen-cli --test golden`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use libtest_mimic::{Arguments, Failed, Trial};
use serde_json::Value;

const REDPEN: &str = env!("CARGO_BIN_EXE_redpen");

fn main() {
    let args = Arguments::from_args();
    let root = repo_root();
    let cases = root.join("tests").join("cases");
    let filter = std::env::var("GOLDEN_FILTER")
        .ok()
        .filter(|f| !f.is_empty());
    let update = std::env::var("UPDATE_GOLDEN")
        .map(|v| v == "1")
        .unwrap_or(false);

    let mut trials = Vec::new();
    for module_dir in sorted_dirs(&cases) {
        let module = file_name(&module_dir);
        for case_dir in sorted_dirs(&module_dir) {
            if !case_dir.join("cmd.json").is_file() {
                continue;
            }
            let id = format!("{module}/{}", file_name(&case_dir));
            let selected = filter.as_deref().is_none_or(|f| id.starts_with(f));
            let overwrite = update && filter.as_deref() == Some(id.as_str());
            let trial_id = id.clone();
            trials.push(
                Trial::test(id, move || {
                    run_case(&case_dir, &trial_id, update, overwrite)
                })
                .with_ignored_flag(!selected),
            );
        }
    }

    libtest_mimic::run(&args, trials).exit();
}

/// The repository root: the first ancestor of this crate that holds the workspace Cargo.toml.
fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if let Ok(text) = fs::read_to_string(dir.join("Cargo.toml"))
            && text.contains("[workspace]")
        {
            return dir;
        }
        assert!(dir.pop(), "no workspace Cargo.toml above the crate");
    }
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    dirs.sort();
    dirs
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[derive(serde::Deserialize)]
struct Cmd {
    args: Vec<String>,
    #[serde(default)]
    exit: i32,
}

fn run_case(case: &Path, id: &str, update: bool, overwrite: bool) -> Result<(), Failed> {
    let cmd_text =
        fs::read_to_string(case.join("cmd.json")).map_err(|e| format!("cmd.json: {e}"))?;
    let cmd: Cmd = serde_json::from_str(&cmd_text).map_err(|e| format!("cmd.json: {e}"))?;

    let output = Command::new(REDPEN)
        .args(&cmd.args)
        .current_dir(case)
        .output()
        .map_err(|e| format!("could not start the redpen binary: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code();
    if code != Some(cmd.exit) {
        return Err(format!(
            "redpen {} exited with {code:?}, expected {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            cmd.args.join(" "),
            cmd.exit
        )
        .into());
    }

    let actual: Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("stdout is not JSON ({e}):\n{stdout}\nstderr:\n{stderr}"))?;

    let expected_path = case.join("expected.json");
    if overwrite || (update && !expected_path.exists()) {
        let mut text = serde_json::to_string_pretty(&actual).expect("JSON serialises");
        text.push('\n');
        fs::write(&expected_path, text)
            .map_err(|e| format!("could not write expected.json: {e}"))?;
        return Ok(());
    }
    let expected_text = fs::read_to_string(&expected_path).map_err(|e| {
        format!(
            "expected.json could not be read ({e}); UPDATE_GOLDEN=1 GOLDEN_FILTER={id} writes it"
        )
    })?;
    let expected: Value = serde_json::from_str(&expected_text)
        .map_err(|e| format!("expected.json is not JSON: {e}"))?;

    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "stdout differs from expected.json\nexpected:\n{}\nactual:\n{}\nstderr:\n{stderr}",
            serde_json::to_string_pretty(&expected).expect("JSON serialises"),
            serde_json::to_string_pretty(&actual).expect("JSON serialises"),
        )
        .into())
    }
}
