//! `redpen-testwin`: the window the Windows capture tests type into (docs/TESTING.md §1).
//!
//! It shows a normal edit box and an `ES_PASSWORD` edit box, prints `ready` once both are on the
//! screen, and then reads one command per line on stdin and answers each with one line on stdout:
//!
//! | Command | Does | Answer |
//! |---|---|---|
//! | `focus normal` / `focus password` | brings the window to the front and focuses that box; later commands act on it | `ok` |
//! | `set <text>` | replaces the focused box's text (the rest of the line, possibly empty) | `ok` |
//! | `select <start> <end>` | selects UTF-16 units `start` to `end` of the focused box | `ok` |
//! | `get` / `get normal` / `get password` | reads the focused (or named) box's text | the text |
//! | `rect` / `rect normal` / `rect password` | the box's place on the screen | `left top right bottom` |
//! | `hwnd` | the window's handle, as `WindowRef::id` takes it | a decimal number |
//! | `quit` | closes the window and exits | `ok` |
//!
//! Anything else answers `error: <what went wrong>`. The window closes when stdin ends.

#[cfg(not(windows))]
fn main() {
    eprintln!("redpen-testwin runs on Windows only.");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(windows)]
mod app {
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW,
        ES_AUTOHSCROLL, ES_PASSWORD, GetForegroundWindow, GetMessageW, GetWindowRect,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, HMENU, MSG, PostMessageW,
        PostQuitMessage, RegisterClassW, SW_SHOW, SendMessageW, SetForegroundWindow,
        SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
        WM_CLOSE, WM_DESTROY, WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_TABSTOP,
        WS_VISIBLE,
    };
    use windows::core::{HSTRING, w};

    /// `EM_SETSEL`: selects a range of an edit box.
    const EM_SETSEL: u32 = 0x00B1;

    static MAIN: AtomicIsize = AtomicIsize::new(0);
    static NORMAL: AtomicIsize = AtomicIsize::new(0);
    static PASSWORD: AtomicIsize = AtomicIsize::new(0);
    /// Whether commands act on the password box.
    static ON_PASSWORD: AtomicBool = AtomicBool::new(false);

    fn hwnd(slot: &AtomicIsize) -> HWND {
        HWND(slot.load(Ordering::SeqCst) as *mut core::ffi::c_void)
    }

    /// One command, run on the window's thread, and its answer.
    struct Request {
        line: String,
        answer: String,
    }

    pub fn run() {
        // SAFETY: the window class, the windows and the message loop all live on this thread;
        // the window procedure only touches them from here.
        unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                lpszClassName: w!("RedpenTestWindow"),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                fail("the window class could not be registered");
            }
            let main = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("RedpenTestWindow"),
                w!("Redpen test window"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                480,
                160,
                None,
                None,
                None,
                None,
            )
            .unwrap_or_else(|_| fail("the window could not be created"));
            MAIN.store(main.0 as isize, Ordering::SeqCst);
            let edit_style = WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP;
            let normal = edit(main, edit_style, 10, 1);
            let password = edit(main, edit_style | WINDOW_STYLE(ES_PASSWORD as u32), 50, 2);
            NORMAL.store(normal.0 as isize, Ordering::SeqCst);
            PASSWORD.store(password.0 as isize, Ordering::SeqCst);
            let _ = ShowWindow(main, SW_SHOW);
            bring_to_front(main, normal);
        }
        say("ready");
        std::thread::spawn(read_commands);
        let mut msg = MSG::default();
        // SAFETY: the standard message loop of this thread's windows.
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// One edit box, `top` pixels down the window.
    unsafe fn edit(parent: HWND, style: WINDOW_STYLE, top: i32, id: usize) -> HWND {
        // SAFETY: `parent` is a live window of this thread.
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!(""),
                style | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                10,
                top,
                440,
                28,
                Some(parent),
                Some(HMENU(id as *mut core::ffi::c_void)),
                None,
                None,
            )
        }
        .unwrap_or_else(|_| fail("an edit box could not be created"))
    }

    /// Brings the window to the front and focuses `edit`. Joins the input of the window in
    /// front for the moment it takes, which Windows requires before it lets a background
    /// process take the front.
    unsafe fn bring_to_front(main: HWND, edit: HWND) {
        // SAFETY: plain calls on live windows; the input queues are detached again at once.
        unsafe {
            let me = GetCurrentThreadId();
            let front = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let attached =
                front != 0 && front != me && AttachThreadInput(me, front, true).as_bool();
            let _ = SetForegroundWindow(main);
            let _ = BringWindowToTop(main);
            let _ = SetFocus(Some(edit));
            if attached {
                let _ = AttachThreadInput(me, front, false);
            }
        }
    }

    fn read_commands() {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let line = line.trim_end_matches('\r').to_string();
            let quit = line.trim() == "quit";
            let mut request = Request {
                line,
                answer: String::new(),
            };
            // SAFETY: the window procedure fills `request` before SendMessageW returns.
            unsafe {
                SendMessageW(
                    hwnd(&MAIN),
                    WM_APP,
                    Some(WPARAM(&mut request as *mut Request as usize)),
                    None,
                );
            }
            say(&request.answer);
            if quit {
                return;
            }
        }
        // SAFETY: posting to a window of this process.
        unsafe {
            let _ = PostMessageW(Some(hwnd(&MAIN)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_APP => {
                // SAFETY: WM_APP is sent only by `read_commands`, with a live Request.
                let request = unsafe { &mut *(wparam.0 as *mut Request) };
                request.answer = run_command(&request.line);
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: ends this thread's message loop.
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // SAFETY: the default handling of every other message.
            _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
    }

    fn target(name: Option<&str>) -> Result<HWND, String> {
        match name.map(str::trim) {
            None | Some("") => Ok(if ON_PASSWORD.load(Ordering::SeqCst) {
                hwnd(&PASSWORD)
            } else {
                hwnd(&NORMAL)
            }),
            Some("normal") => Ok(hwnd(&NORMAL)),
            Some("password") => Ok(hwnd(&PASSWORD)),
            Some(other) => Err(format!("error: no box named {other}")),
        }
    }

    fn run_command(line: &str) -> String {
        let (command, rest) = line.split_once(' ').unwrap_or((line.trim(), ""));
        let result = match command {
            "focus" => match rest.trim() {
                "normal" | "password" => {
                    ON_PASSWORD.store(rest.trim() == "password", Ordering::SeqCst);
                    target(None).map(|edit| {
                        // SAFETY: both windows belong to this thread.
                        unsafe { bring_to_front(hwnd(&MAIN), edit) };
                        "ok".to_string()
                    })
                }
                other => Err(format!(
                    "error: focus takes normal or password, not {other}"
                )),
            },
            "set" => target(None).and_then(|edit| {
                // SAFETY: `edit` belongs to this thread; the text outlives the call.
                unsafe { SetWindowTextW(edit, &HSTRING::from(rest)) }
                    .map(|()| "ok".to_string())
                    .map_err(|e| format!("error: {e}"))
            }),
            "select" => {
                let numbers: Vec<usize> = rest
                    .split_whitespace()
                    .filter_map(|n| n.parse().ok())
                    .collect();
                match numbers[..] {
                    [start, end] => target(None).map(|edit| {
                        // SAFETY: `edit` belongs to this thread.
                        unsafe {
                            SendMessageW(
                                edit,
                                EM_SETSEL,
                                Some(WPARAM(start)),
                                Some(LPARAM(end as isize)),
                            )
                        };
                        "ok".to_string()
                    }),
                    _ => Err("error: select takes <start> <end>".to_string()),
                }
            }
            "get" => target(Some(rest)).map(|edit| {
                // SAFETY: the buffer is as long as the text plus its NUL.
                unsafe {
                    let len = GetWindowTextLengthW(edit).max(0) as usize;
                    let mut buffer = vec![0u16; len + 1];
                    let copied = GetWindowTextW(edit, &mut buffer).max(0) as usize;
                    String::from_utf16_lossy(&buffer[..copied])
                }
            }),
            "rect" => target(Some(rest)).and_then(|edit| {
                let mut r = RECT::default();
                // SAFETY: `r` outlives the call.
                unsafe { GetWindowRect(edit, &mut r) }
                    .map(|()| format!("{} {} {} {}", r.left, r.top, r.right, r.bottom))
                    .map_err(|e| format!("error: {e}"))
            }),
            "hwnd" => Ok((hwnd(&MAIN).0 as usize).to_string()),
            "quit" => {
                // SAFETY: the main window belongs to this thread.
                unsafe {
                    let _ = PostMessageW(Some(hwnd(&MAIN)), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                Ok("ok".to_string())
            }
            other => Err(format!("error: unknown command {other}")),
        };
        result.unwrap_or_else(|e| e)
    }

    fn say(line: &str) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    fn fail(why: &str) -> ! {
        eprintln!("redpen-testwin: {why}");
        std::process::exit(1);
    }
}
