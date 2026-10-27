//! Redpen's core: the platform-free library behind the tray app and the `redpen` command-line
//! tool (docs/ARCHITECTURE.md §3).
//!
//! The core never reads the clock, randomness, the filesystem or the network directly
//! (invariant 10): time comes through [`clock::Clock`] and [`clock::Sleeper`], and every HTTP
//! request goes through [`net::HttpTransport`], which tests replace with [`net::FakeTransport`].
//! Checking, rewriting, providers, capture and storage arrive with the later modules
//! (docs/ROADMAP.md).

pub mod clock;
pub mod net;
