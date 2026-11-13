//! Build script: Tauri's own set-up, and the publik API app token (docs/modules/03-publik.md).
//!
//! The token is public by design and committed as `publik-app-token.txt` beside this file
//! (docs/PUBLIK-CONTRACT.md §2); `PUBLIK_APP_TOKEN` in the build environment overrides it. The
//! first line that is neither empty nor a `#` comment is compiled in as
//! `REDPEN_PUBLIK_APP_TOKEN`; `publik.rs` accepts it only if it starts with `pat_`, so the
//! placeholder file (one comment line) builds an app that says it has no token.

use std::fs;

const TOKEN_FILE: &str = "publik-app-token.txt";

fn main() {
    println!("cargo:rerun-if-changed={TOKEN_FILE}");
    println!("cargo:rerun-if-env-changed=PUBLIK_APP_TOKEN");
    let from_env = std::env::var("PUBLIK_APP_TOKEN")
        .ok()
        .and_then(|v| first_token_line(&v));
    let token = from_env
        .or_else(|| {
            fs::read_to_string(TOKEN_FILE)
                .ok()
                .and_then(|v| first_token_line(&v))
        })
        .unwrap_or_default();
    println!("cargo:rustc-env=REDPEN_PUBLIK_APP_TOKEN={token}");
    tauri_build::build()
}

/// The first line that is neither empty nor a comment, trimmed.
fn first_token_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
}
