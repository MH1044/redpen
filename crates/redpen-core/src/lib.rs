//! Redpen's core: the platform-free library behind the tray app and the `redpen` command-line
//! tool (docs/ARCHITECTURE.md §3).
//!
//! The core never reads the clock, randomness, the filesystem or the network directly
//! (invariant 10): time comes through [`clock::Clock`] and [`clock::Sleeper`], every HTTP
//! request goes through [`net::HttpTransport`], which tests replace with [`net::FakeTransport`],
//! and only [`store`] touches files, under the directories its caller names.
//! [`check`] runs Harper on this computer (module 01); [`provider`] is the one chat client for
//! the own key, a local model and the fake provider, with its cache, usage counts and errors, and
//! [`store`] holds the settings, the keys and `redpen.db` (module 02). Rewriting, publik API,
//! capture and the plagiarism check arrive with the later modules (docs/ROADMAP.md).

pub mod check;
pub mod clock;
pub mod net;
pub mod provider;
pub mod store;
