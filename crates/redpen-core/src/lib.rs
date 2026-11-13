//! Redpen's core: the platform-free library behind the tray app and the `redpen` command-line
//! tool (docs/ARCHITECTURE.md §3).
//!
//! The core never reads the clock, randomness, the filesystem or the network directly
//! (invariant 10): time comes through [`clock::Clock`] and [`clock::Sleeper`], every HTTP
//! request goes through [`net::HttpTransport`], which tests replace with [`net::FakeTransport`],
//! and only [`store`] touches files, under the directories its caller names.
//! [`check`] runs Harper on this computer (module 01); [`provider`] is the one chat client for
//! the own key, a local model and the fake provider, with its cache, usage counts and errors, and
//! [`store`] holds the settings, the keys and `redpen.db` (module 02). [`platform`] is the
//! capture boundary, the `Capture` trait with its refusals and fake, and [`flow`] drives it: the
//! capture of a selection, Accept and Undo (module 05). For publik API (module 03), [`provider`]
//! adds the tier rule, the idempotency key, the mapping of its errors, the resolver trait the app
//! implements and the web search answer; the publik credential itself is the app's. Rewriting
//! and the plagiarism check arrive with the later modules (docs/ROADMAP.md).

pub mod check;
pub mod clock;
pub mod flow;
pub mod net;
pub mod platform;
pub mod provider;
pub mod store;
