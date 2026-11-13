//! `publik-proof`: the maintainer's one real publik API call (`scripts/publik-proof.sh`,
//! docs/modules/03-publik.md). It spends money: it shows the disclosure and asks before it sets
//! up this computer, and asks again before a web search. Never run by the tests or CI.

use std::io::Write;

use redpen_app::proof::{self, USAGE};
use redpen_app::publik::PublikOptions;

fn main() {
    let opts = match proof::parse_args(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(_) => {
            eprintln!("publik-proof could not start its runtime. {USAGE}");
            std::process::exit(1);
        }
    };
    let transport = redpen_app::production_transport();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    let code = runtime.block_on(proof::run(
        &transport,
        &opts,
        &PublikOptions::current(),
        &mut input,
        &mut out,
    ));
    let _ = out.flush();
    std::process::exit(code);
}
