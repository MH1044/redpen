//! `redpen-app --measure-popup` (docs/modules/06-popup-and-settings-ui.md, Acceptance test 17):
//! how long the check shortcut takes from the press to the findings, on a real Windows desktop.
//!
//! It starts the helper window `redpen-testwin` (built next to `redpen-app`, or named by
//! `REDPEN_TESTWIN`), gives it a 300-word text, and runs the shortcut's path 20 times: the
//! capture of the selection from the window in front (the refusals, the key wait, Ctrl+C and the
//! clipboard restored) and Harper's check, through the same [`Sessions::open`] the shortcut
//! calls. It prints `{"p95_ms": N}` (nearest rank) and exits 0 when N is at most 300, 1 when it
//! is above, and 2 when it could not measure. Nothing is sent anywhere: the chat client has a
//! transport with no answers.

use std::time::Duration;

/// How many captures are timed.
pub const RUNS: usize = 20;
/// The target: the popup has its findings within 300 ms (docs/PRODUCT.md §7).
pub const TARGET_MS: u64 = 300;

/// The text the helper window holds: 300 words of ordinary prose with two mistakes, so Harper
/// has findings to make.
pub fn text() -> String {
    ["Their going to send the report tomorow, and the team will check every number before the meeting starts on Friday morning so that nobody is surprised by all the results."; 10]
        .join(" ")
}

/// The 95th percentile of `samples` by the nearest-rank method: the value at rank
/// ceil(0.95 × n) of the sorted samples (the 19th of 20). `None` for no samples.
pub fn p95(samples: &[Duration]) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let mut ms: Vec<u64> = samples
        .iter()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .collect();
    ms.sort_unstable();
    let rank = (ms.len() * 95).div_ceil(100);
    Some(ms[rank.max(1) - 1])
}

/// The line `--measure-popup` prints.
pub fn report(p95_ms: u64) -> String {
    format!("{{\"p95_ms\": {p95_ms}}}")
}

/// Runs the measurement and returns the process's exit code.
pub fn run() -> i32 {
    #[cfg(windows)]
    {
        match desktop::measure() {
            Ok(samples) => {
                let p = p95(&samples).unwrap_or(u64::MAX);
                println!("{}", report(p));
                if p <= TARGET_MS { 0 } else { 1 }
            }
            Err(e) => {
                eprintln!("redpen-app --measure-popup: {e}");
                2
            }
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("redpen-app --measure-popup runs on Windows only.");
        2
    }
}

#[cfg(windows)]
mod desktop {
    use std::io::{BufRead, BufReader, Write};
    use std::path::PathBuf;
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::Arc;
    use std::sync::mpsc::{self, Receiver};
    use std::time::{Duration, Instant};

    use redpen_core::clock::{SystemClock, TokioSleeper};
    use redpen_core::flow::Sessions;
    use redpen_core::net::FakeTransport;
    use redpen_core::provider::{AiClient, AiError, FixedResolver, NoopStatusSink};
    use redpen_core::store::{Db, Settings};
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, GetForegroundWindow, GetWindowThreadProcessId,
    };

    use super::{RUNS, text};

    /// The helper window, driven one line at a time (its protocol is in
    /// `crates/redpen-os/src/bin/redpen-testwin.rs`).
    struct TestWin {
        child: Child,
        stdin: ChildStdin,
        lines: Receiver<String>,
    }

    impl TestWin {
        fn start() -> Result<Self, String> {
            let path = std::env::var_os("REDPEN_TESTWIN")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::current_exe()
                        .ok()
                        .map(|exe| exe.with_file_name("redpen-testwin.exe"))
                })
                .ok_or("the helper window's path is unknown")?;
            let mut child = Command::new(&path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|_| {
                    "redpen-testwin could not be started; build it with cargo build -p redpen-os --bin redpen-testwin".to_string()
                })?;
            // SAFETY: a plain process id; failure only means the helper may not come forward.
            let _ = unsafe { AllowSetForegroundWindow(child.id()) };
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let stdout = child.stdout.take().ok_or("no stdout")?;
            let (tx, lines) = mpsc::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line.trim_end_matches('\r').to_string()).is_err() {
                        break;
                    }
                }
            });
            let mut w = Self {
                child,
                stdin,
                lines,
            };
            match w.reply()?.as_str() {
                "ready" => Ok(w),
                other => Err(format!("redpen-testwin said {other:?} instead of ready")),
            }
        }

        fn reply(&mut self) -> Result<String, String> {
            self.lines
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| "redpen-testwin gave no answer within 10 s".to_string())
        }

        fn cmd(&mut self, line: &str) -> Result<String, String> {
            writeln!(self.stdin, "{line}")
                .and_then(|()| self.stdin.flush())
                .map_err(|_| "redpen-testwin stopped reading".to_string())?;
            let reply = self.reply()?;
            if reply.starts_with("error") {
                return Err(format!("redpen-testwin refused a command: {reply}"));
            }
            Ok(reply)
        }

        /// Waits until the helper's window is in front.
        fn wait_front(&self) -> Result<(), String> {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut pid = 0u32;
                // SAFETY: GetForegroundWindow takes nothing; the pid pointer lives for the call.
                unsafe {
                    let hwnd = GetForegroundWindow();
                    if !hwnd.is_invalid() {
                        GetWindowThreadProcessId(hwnd, Some(&mut pid));
                    }
                }
                if pid == self.child.id() {
                    return Ok(());
                }
                if Instant::now() > deadline {
                    return Err("redpen-testwin's window did not come to the front".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    impl Drop for TestWin {
        fn drop(&mut self) {
            let _ = writeln!(self.stdin, "quit");
            let _ = self.child.wait();
        }
    }

    /// Times [`RUNS`] captures and checks of the helper's selected text.
    pub fn measure() -> Result<Vec<Duration>, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_| "no runtime".to_string())?;
        let clock = Arc::new(SystemClock);
        let sleeper = Arc::new(TokioSleeper);
        let ai = Arc::new(AiClient::new(
            Arc::new(FakeTransport::scripted(Vec::new())),
            Db::open_in_memory().map_err(|_| "no database".to_string())?,
            clock.clone(),
            sleeper.clone(),
            Arc::new(NoopStatusSink),
        ));
        let sessions = Sessions::new(
            crate::system_capture(),
            ai,
            Arc::new(FixedResolver::failing(AiError::NeedsConsent)),
            clock,
            sleeper,
        );
        let settings = Settings::default();
        // Harper is built at start, before any press of the shortcut.
        sessions.warm(&settings);

        let text = text();
        let units = text.encode_utf16().count();
        let mut win = TestWin::start()?;
        win.cmd("focus normal")?;
        win.cmd(&format!("set {text}"))?;
        let mut samples = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            win.cmd("focus normal")?;
            win.cmd(&format!("select 0 {units}"))?;
            win.wait_front()?;
            let started = Instant::now();
            let session = runtime
                .block_on(sessions.open(&settings))
                .map_err(|e| format!("the capture stopped: {}", e.kind()))?;
            let elapsed = started.elapsed();
            if session.text != text {
                return Err("the capture read another text".into());
            }
            sessions.close_all();
            samples.push(elapsed);
        }
        Ok(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_is_three_hundred_words() {
        assert_eq!(redpen_core::words::words(&text()), 300);
    }

    #[test]
    fn p95_is_the_nearest_rank() {
        let ms: Vec<Duration> = (1..=20).map(Duration::from_millis).collect();
        assert_eq!(p95(&ms), Some(19));
        let reversed: Vec<Duration> = ms.iter().rev().copied().collect();
        assert_eq!(p95(&reversed), Some(19));
        assert_eq!(p95(&[Duration::from_millis(7)]), Some(7));
        assert_eq!(p95(&[]), None);
    }

    #[test]
    fn the_report_is_one_json_line() {
        assert_eq!(report(212), r#"{"p95_ms": 212}"#);
        let v: serde_json::Value = serde_json::from_str(&report(212)).unwrap();
        assert_eq!(v["p95_ms"], 212);
    }
}
