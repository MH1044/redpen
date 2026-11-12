//! The gate scripts' own failures (docs/TESTING.md §4, docs/modules/01-harper-check.md):
//! `scripts/gates.sh` and `scripts/gates.ps1` refuse an unknown gate with
//! `error: unknown gate <name>.` and the usage, and a missing gate with the usage, both with exit
//! code 2. Neither path builds anything, so the scripts run here as they are.
//!
//! Each script runs with its own shell where that shell is installed: `sh` for `gates.sh`;
//! Windows PowerShell on Windows, or PowerShell 7 (`pwsh`) elsewhere, for `gates.ps1`. At least
//! one of them is always there (`sh` on Linux, `powershell` on Windows), and the test fails if
//! neither ran.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::{Command, Output};

/// The repository root: the first ancestor of this crate that holds the workspace Cargo.toml.
fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml"))
            && text.contains("[workspace]")
        {
            return dir;
        }
        assert!(dir.pop(), "no workspace Cargo.toml above the crate");
    }
}

/// A gate script run with `shell` and `args` from the repository root, or `None` when `shell`
/// is not installed.
fn run(shell: &str, args: &[&str]) -> Option<Output> {
    match Command::new(shell)
        .args(args)
        .current_dir(repo_root())
        .output()
    {
        Ok(output) => Some(output),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("{shell} is not installed here; its gate script is not run");
            None
        }
        Err(e) => panic!("{shell} could not be started: {e}"),
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Runs `script` with `shell` (and `shell_args` before it) for a missing gate, an unknown gate
/// and `--help`, and checks each exit code and message. `false` when the shell is not installed.
fn check_script(shell: &str, shell_args: &[&str], script: &str, usage: &str) -> bool {
    let with = |gate_args: &[&str]| {
        let mut args: Vec<&str> = shell_args.to_vec();
        args.push(script);
        args.extend_from_slice(gate_args);
        run(shell, &args)
    };

    let Some(unknown) = with(&["bogus"]) else {
        return false;
    };
    // gates.sh writes failures to standard error; gates.ps1 writes them with Write-Host, which
    // reaches standard output when run with -File. Both are read together.
    let said = format!("{}{}", text(&unknown.stdout), text(&unknown.stderr));
    assert_eq!(unknown.status.code(), Some(2), "{script} bogus: {said}");
    assert!(
        said.contains("error: unknown gate bogus.\n"),
        "{script} bogus: {said}"
    );
    assert!(said.contains(usage), "{script} bogus: {said}");
    assert!(
        said.contains("Gates: check-speed"),
        "{script} bogus: {said}"
    );

    let missing = with(&[]).expect("the shell ran a moment ago");
    let said = format!("{}{}", text(&missing.stdout), text(&missing.stderr));
    assert_eq!(missing.status.code(), Some(2), "{script}: {said}");
    assert!(said.contains(usage), "{script}: {said}");
    assert!(!said.contains("error: unknown gate"), "{script}: {said}");

    let help = with(&["--help"]).expect("the shell ran a moment ago");
    let said = text(&help.stdout);
    assert_eq!(help.status.code(), Some(0), "{script} --help: {said}");
    assert!(said.contains(usage), "{script} --help: {said}");
    true
}

#[test]
fn an_unknown_or_missing_gate_exits_2_with_the_usage() {
    let sh = check_script(
        "sh",
        &[],
        "scripts/gates.sh",
        "Usage: sh scripts/gates.sh <gate> [options]",
    );
    let (powershell, flags): (&str, &[&str]) = if cfg!(windows) {
        (
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ],
        )
    } else {
        ("pwsh", &["-NoProfile", "-NonInteractive", "-File"])
    };
    let ps = check_script(
        powershell,
        flags,
        "scripts/gates.ps1",
        "Usage: powershell -File scripts/gates.ps1 <gate> [options]",
    );
    assert!(
        sh || ps,
        "neither sh nor PowerShell is installed to run a gate script"
    );
}
