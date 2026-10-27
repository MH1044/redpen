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
| secrecy | Apache-2.0 OR MIT | Header values that hold a key and never print it |
| serde | MIT OR Apache-2.0 | Serialisation of recordings, settings and command results |
| serde_json | MIT OR Apache-2.0 | JSON bodies, recordings and the command-line tool's output |
| sha2 | MIT OR Apache-2.0 | SHA-256 of canonical requests (later: cache keys) |
| thiserror | MIT OR Apache-2.0 | Error types with their messages |
| tokio | MIT | The async runtime and the production `Sleeper` |
| ignore (tests only) | Unlicense OR MIT | The source-hygiene test's fallback walk, honouring `.gitignore` |
| libtest-mimic (tests only) | MIT OR Apache-2.0 | One test per golden case directory |
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

None. Redpen ships no model weights: checking uses Harper's rules (module 01), and rewrites go to
the provider the user chose.

## Datasets

None yet. The test corpora are written for this repository (CC0) at the module that first needs
them; the plagiarism corpus (module 09) quotes public-domain works, each with its source.

## Other

| Item | Licence | Note |
|---|---|---|
| `app/src-tauri/icons/*` | Project's own (MIT OR Apache-2.0) | Placeholder images written by `scripts/make-icons.mjs` until the packaging module designs the real icons |
