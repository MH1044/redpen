# Third-party software, models and data

Every direct dependency, with its licence and source. Transitive Rust dependencies are checked by
the licence gate (`deny.toml`) on every build. Models and datasets get a row here when a module
adds them.

## Rust crates (crates.io)

| Crate | Licence | Used for |
|---|---|---|
| tauri | Apache-2.0 OR MIT | The desktop shell: tray, windows, commands |
| tauri-build | Apache-2.0 OR MIT | Build-time setup of the Tauri crate |
| tauri-plugin-global-shortcut | Apache-2.0 OR MIT | The check shortcut, registered with the operating system from Rust only (module 06); it brings `tauri-plugin` (Apache-2.0 OR MIT), `global-hotkey` (Apache-2.0 OR MIT), `keyboard-types` (MIT OR Apache-2.0) and, on Linux, `x11rb` and `x11rb-protocol` (MIT OR Apache-2.0), `xkeysym` (MIT OR Apache-2.0 OR Zlib) and `gethostname` (Apache-2.0), which `x11rb` uses to find the X server's authorisation entry; Redpen never reads or sends the computer's name |
| async-trait | MIT OR Apache-2.0 | The async `HttpTransport` and `Sleeper` traits |
| harper-core | Apache-2.0 | Grammar, spelling and style checking on this computer, pinned at `=3.0.0` (module 01); it brings Harper's `harper-brill`, `harper-pos-utils` and `harper-thesaurus` (Apache-2.0) |
| unicode-segmentation | MIT OR Apache-2.0 | UAX #29 words: the 1,000-word text of the `check-speed` gate; a rewrite's word count, its `max_tokens`, the minimal-edit guard and the word diff (module 04) |
| jsonschema | MIT | Every AI answer checked against its schema before use (module 04), built with default features off so it never fetches a remote `$ref` or reads a file; it brings `referencing`, `fancy-regex`, `fraction`, `email_address` and their companions under permissive licences the licence gate checks, among them `borrow-or-share` (MIT-0, MIT without the notice condition) through `fluent-uri` |
| regex | MIT OR Apache-2.0 | Redaction's patterns and the cleaning of AI answers (module 04); the source-hygiene test's patterns |
| similar | Apache-2.0 | Myers' diff for the word diff and the minimal-edit guard (module 04), called without a deadline so it never reads the clock |
| whatlang | MIT | Language keeping: whether a rewrite is in the language of its text (module 04) |
| reqwest | MIT OR Apache-2.0 | The production HTTP transport for every provider (module 02), with its default TLS: rustls, certificates checked against the operating system's store by `rustls-platform-verifier` (MIT OR Apache-2.0), cryptography from `aws-lc-rs` (ISC AND (Apache-2.0 OR ISC)) |
| rusqlite | MIT | `redpen.db`: the answer cache, daily usage and the sent log (module 02); with `bundled`, it compiles in SQLite itself through `libsqlite3-sys` (MIT), and SQLite's own source is in the public domain |
| secrecy | Apache-2.0 OR MIT | Keys that never print: header values, the resolved provider, `secrets.json` |
| serde | MIT OR Apache-2.0 | Serialisation of recordings, settings and command results |
| serde_json | MIT OR Apache-2.0 | JSON bodies, recordings and the command-line tool's output |
| sha2 | MIT OR Apache-2.0 | SHA-256 of canonical requests and of the answer cache's keys |
| thiserror | MIT OR Apache-2.0 | Error types with their messages |
| tokio | MIT | The async runtime (the command line's `chat` too) and the production `Sleeper` |
| tracing | MIT | Log events without content: names, durations, sizes, error kinds and HTTP status codes (module 02) |
| windows | MIT OR Apache-2.0 | Capture on Windows (module 05), in `redpen-os`: UI Automation for password fields, process tokens for elevation, `SendInput` for Ctrl+C, Ctrl+V and Ctrl+Z, and the clipboard; in the app on Windows (module 03), `ShellExecuteW` to hand a publik account link to the system browser and `AttachConsole` so a release build's self-test line reaches the console it was started from, and (module 06) `ToUnicodeEx`, which tells whether a new check shortcut types a character on the keyboard layout in use; it brings Microsoft's `windows-core`, `windows-result`, `windows-strings`, `windows-link` and their companions (MIT OR Apache-2.0) |
| uuid | Apache-2.0 OR MIT | A random version 4 install id for publik API's `POST /installs` (module 03), in the app only; with its `v4` feature it draws on `getrandom` (MIT OR Apache-2.0), both already in the build through Tauri |
| ignore (tests only) | Unlicense OR MIT | The source-hygiene test's fallback walk, honouring `.gitignore` |
| libtest-mimic (tests only) | MIT OR Apache-2.0 | One test per golden case directory |
| proptest (tests only) | MIT OR Apache-2.0 | Generated texts with a fixed seed in the checking tests (module 01) |
| tempfile (tests only) | MIT OR Apache-2.0 | Temporary directories in tests |
| toml (tests only) | MIT OR Apache-2.0 | Reading `deny.toml` in tests |
| tracing-subscriber (tests only) | MIT | A capturing subscriber, so tests can prove no key reaches a log line (module 02; the app's publik API tests from module 03) |

## npm packages (registry.npmjs.org)

| Package | Licence | Used for |
|---|---|---|
| react, react-dom | MIT | The interface |
| @tauri-apps/api | Apache-2.0 OR MIT | Calling commands from the web view |
| vite, @vitejs/plugin-react | MIT | Building the front end |
| typescript | Apache-2.0 | Type checking |
| vitest, jsdom | MIT | Interface unit tests |
| @testing-library/react, @testing-library/dom, @testing-library/jest-dom | MIT | Rendering and querying components in tests |
| eslint, @eslint/js, typescript-eslint, eslint-plugin-react-hooks, eslint-plugin-react-refresh, globals | MIT | Lint |
| prettier | MIT | Formatting the front end |
| @types/node, @types/react, @types/react-dom | MIT | Type definitions |

## Build and CI tools (not shipped)

| Tool | Licence | Used for |
|---|---|---|
| cargo-deny | MIT OR Apache-2.0 | The Rust licence gate |
| actions/checkout, actions/setup-node | MIT | Continuous integration |
| Swatinem/rust-cache | MIT | Caching Cargo builds in continuous integration |
| taiki-e/install-action | Apache-2.0 OR MIT | Installing cargo-deny in continuous integration |

## Models

Redpen trains no model and ships no weights of its own; rewrites go to the provider the user chose.
Harper (module 01) compiles its own data into the crates it is built from, and Redpen ships it
unchanged as part of `harper-core`:

| Item | Licence | Source | Used for |
|---|---|---|---|
| Harper's part-of-speech tagger and noun-phrase chunker: Brill rule sets and a small Burn model (`finished_chunker/model.mpk`, about 0.8 MB) | Apache-2.0 | the `harper-brill` crate, 3.0.0, from the Harper project | Tagging words so Harper's grammar rules can match |
| Harper's thesaurus and word-frequency list | Apache-2.0 | the `harper-thesaurus` crate, 3.0.0, from the Harper project | Word-choice suggestions |
| Harper's curated English dictionary | Apache-2.0 | `harper-core` 3.0.0 | Spelling |

Harper's repository does not name the corpus its tagger and chunker were trained on.

## Datasets

None yet. The test corpora are written for this repository (CC0) at the module that first needs
them; the plagiarism corpus (module 09) quotes public-domain works, each with its source.

## Other

| Item | Licence | Note |
|---|---|---|
| `app/src-tauri/icons/*` | Project's own (MIT OR Apache-2.0) | Placeholder images written by `scripts/make-icons.mjs` until the packaging module designs the real icons |
