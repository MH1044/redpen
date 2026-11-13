//! The platform boundary (docs/ARCHITECTURE.md §3): the [`Capture`] trait that reads a selection
//! and types text back, the refusals of [`check_allowed`], and [`FakeCapture`] for tests. The
//! Windows implementation is in `redpen-os` (module 05); Linux follows in module 07.

pub mod capture;
pub mod fake;

pub use capture::{Capture, CaptureError, Foreground, Point, Rect, WindowRef, check_allowed};
pub use fake::{CaptureCall, FakeCapture};
