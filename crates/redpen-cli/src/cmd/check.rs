//! `redpen check [--dialect D] [--disable C]... FILE`: Harper's findings for the file, as
//! `{"findings": [...]}` (docs/modules/01-harper-check.md).

use std::io::Write;

use redpen_core::check::{CheckOptions, Checker, Finding};
use serde::Serialize;

use super::{Accepts, fail, parse, print, read_text, refused};

pub const ACCEPTS: Accepts = Accepts {
    disable: true,
    runs: false,
};

#[derive(Serialize)]
struct Output<'a> {
    findings: &'a [Finding],
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let opts = match parse("check", args, ACCEPTS) {
        Ok(opts) => opts,
        Err(failure) => return fail(failure, err),
    };
    let text = match read_text(opts.file.as_deref().unwrap_or_default()) {
        Ok(text) => text,
        Err(failure) => return fail(failure, err),
    };
    let checker = Checker::new(CheckOptions {
        dialect: opts.dialect,
        disabled: opts.disabled,
    });
    match checker.check(&text) {
        Ok(findings) => print(
            &Output {
                findings: &findings,
            },
            out,
        ),
        Err(e) => refused(&e, out, err),
    }
}
