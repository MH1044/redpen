# Third-party software, models and data

Every direct dependency, with its licence and source. Transitive Rust dependencies are checked by
the licence gate (`deny.toml`) on every build. Models and datasets get a row here when a module
adds them.

## Rust crates (crates.io)

| Crate | Licence | Used for |
|---|---|---|
| tauri | Apache-2.0 OR MIT | The desktop shell: tray, windows, commands |
| tauri-build | Apache-2.0 OR MIT | Build-time setup of the Tauri crate |
| async-trait | MIT OR Apache-2.0 | The async `HttpTransport` and `Sleeper` traits |
| harper-core | Apache-2.0 | Grammar, spelling and style checking on this computer, pinned at `=3.0.0` (module 01); it brings Harper's `harper-brill`, `harper-pos-utils` and `harper-thesaurus` (Apache-2.0) |
| unicode-segmentation | MIT OR Apache-2.0 | UAX #29 words: the 1,000-word text of the `check-speed` gate (later: word counts and sentences) |
| secrecy | Apache-2.0 OR MIT | Header values that hold a key and never print it |
| serde | MIT OR Apache-2.0 | Serialisation of recordings, settings and command results |
| serde_json | MIT OR Apache-2.0 | JSON bodies, recordings and the command-line tool's output |
| sha2 | MIT OR Apache-2.0 | SHA-256 of canonical requests (later: cache keys) |
| thiserror | MIT OR Apache-2.0 | Error types with their messages |
| tokio | MIT | The async runtime and the production `Sleeper` |
| ignore (tests only) | Unlicense OR MIT | The source-hygiene test's fallback walk, honouring `.gitignore` |
| libtest-mimic (tests only) | MIT OR Apache-2.0 | One test per golden case directory |
| proptest (tests only) | MIT OR Apache-2.0 | Generated texts with a fixed seed in the checking tests (module 01) |
| regex (tests only) | MIT OR Apache-2.0 | The source-hygiene test's patterns |
| tempfile (tests only) | MIT OR Apache-2.0 | Temporary directories in tests |
| toml (tests only) | MIT OR Apache-2.0 | Reading `deny.toml` in tests |

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
