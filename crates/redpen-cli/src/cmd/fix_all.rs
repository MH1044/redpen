//! `redpen fix-all [--dialect D] FILE`: the file with the first suggestion of every finding
//! applied until nothing is left to fix, as `{"text": "..."}` (docs/modules/01-harper-check.md).

use std::io::Write;

use redpen_core::check::{CheckOptions, Checker};
use serde::Serialize;

use super::{Accepts, fail, parse, print, read_text, refused};

pub const ACCEPTS: Accepts = Accepts {
    disable: false,
    runs: false,
};

#[derive(Serialize)]
struct Output {
    text: String,
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let opts = match parse("fix-all", args, ACCEPTS) {
        Ok(opts) => opts,
        Err(failure) => return fail(failure, err),
    };
    let text = match read_text(opts.file.as_deref().unwrap_or_default()) {
        Ok(text) => text,
        Err(failure) => return fail(failure, err),
    };
    let checker = Checker::new(CheckOptions {
        dialect: opts.dialect,
        disabled: Vec::new(),
    });
    match checker.fix_all(&text) {
        Ok(text) => print(&Output { text }, out),
        Err(e) => refused(&e, out, err),
    }
}
