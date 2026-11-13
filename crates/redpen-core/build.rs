//! Embeds every file under the repository's `prompts/` folder into the core at compile time
//! (docs/modules/04-rewrite-engine.md), so the app never reads a prompt from disk at run time and
//! the prompt a build sends is the one its tests checked.
//!
//! It writes `$OUT_DIR/prompts.rs` with one `(path, contents)` pair per file, the path relative
//! to `prompts/` with `/` separators (`fix-grammar.v1.md`, `schemas/rewrite-result.v1.json`),
//! sorted so the list is the same on every platform. `rewrite::prompts` includes it.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let prompts = manifest.join("..").join("..").join("prompts");
    println!("cargo:rerun-if-changed={}", prompts.display());

    let mut files = Vec::new();
    collect(&prompts, &prompts, &mut files);
    files.sort();

    let mut out = String::from(
        "/// Every file under `prompts/`: its path relative to that folder, and its contents.\n\
         pub static FILES: &[(&str, &str)] = &[\n",
    );
    for (relative, absolute) in &files {
        println!("cargo:rerun-if-changed={}", absolute.display());
        out.push_str(&format!(
            "    ({relative:?}, include_str!({:?})),\n",
            absolute.display().to_string()
        ));
    }
    out.push_str("];\n");

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("set by cargo"));
    fs::write(out_dir.join("prompts.rs"), out).expect("the generated prompt list is written");
}

/// Every file below `dir`, as (path relative to `root` with `/` separators, absolute path).
fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("the prompts folder {} cannot be read: {e}", dir.display()),
    };
    for entry in entries {
        let path = entry.expect("a prompts folder entry").path();
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("below the prompts folder")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            files.push((relative, path));
        }
    }
}
