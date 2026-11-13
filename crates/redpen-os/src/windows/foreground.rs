//! The window in front (docs/modules/05-capture-and-replace-windows.md, Scope): its handle and
//! process (`GetForegroundWindow`, `GetWindowThreadProcessId`), the lower-case executable name
//! (`QueryFullProcessImageNameW`), whether its focused control is a password field (UI
//! Automation, and the `ES_PASSWORD` style of a classic edit box), whether it runs above
//! Redpen's integrity level (its token; a refusal to say counts as elevated), the caret
//! (`GetGUIThreadInfo`) and the mouse.

use std::sync::OnceLock;

use ::windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, POINT};
use ::windows::Win32::Graphics::Gdi::ClientToScreen;
use ::windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel,
};
use ::windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    ES_PASSWORD, GUITHREADINFO, GWL_STYLE, GetClassNameW, GetCursorPos, GetForegroundWindow,
    GetGUIThreadInfo, GetWindowLongPtrW, GetWindowThreadProcessId, IsWindow,
};
use ::windows::core::PWSTR;
use redpen_core::platform::{CaptureError, Foreground, Point, Rect, WindowRef};

use super::{last_error, to_hwnd, uia, window_ref};

/// Win32's "invalid window handle": no window is in front (the desktop is switching).
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;

/// The window in front now.
pub fn foreground() -> Result<Foreground, CaptureError> {
    // SAFETY: no arguments.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return Err(CaptureError::os(ERROR_INVALID_WINDOW_HANDLE));
    }
    describe(hwnd, true)
}

/// The window `w` names, or `None` when it has closed (or its handle now belongs to another
/// process). The password flag is read only when `w` is in front.
pub fn inspect(w: &WindowRef) -> Result<Option<Foreground>, CaptureError> {
    if !exists(w) {
        return Ok(None);
    }
    let hwnd = to_hwnd(w);
    // SAFETY: no arguments.
    let front = unsafe { GetForegroundWindow() } == hwnd;
    describe(hwnd, front).map(Some)
}

/// Whether the window `w` names still exists and still belongs to its process.
pub fn exists(w: &WindowRef) -> bool {
    let hwnd = to_hwnd(w);
    // SAFETY: IsWindow accepts any value; the process id is read only for a window that exists.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid)) != 0 && pid == w.pid
    }
}

fn describe(hwnd: HWND, front: bool) -> Result<Foreground, CaptureError> {
    let mut pid = 0;
    // SAFETY: `pid` outlives the call.
    let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if thread == 0 {
        return Err(last_error());
    }
    let (process, elevated) = process_facts(pid);
    let focus = gui_thread_info(thread);
    // An elevated window is refused before the password check, and UI Automation could not read
    // it anyway; a window behind others has no keyboard focus to ask about.
    let is_password = if front && !elevated {
        focus
            .as_ref()
            .is_some_and(|f| is_password_edit(f.hwndFocus))
            || uia::focused_is_password()?
    } else {
        false
    };
    Ok(Foreground {
        window: window_ref(hwnd, pid),
        process,
        is_password,
        secure_input: false,
        elevated,
        caret: focus.as_ref().and_then(caret_rect),
        cursor: cursor(),
    })
}

/// The lower-case executable name of process `pid`, and whether it runs above Redpen's
/// integrity level. When Windows refuses to open the process, its name is empty and it counts
/// as elevated; when it refuses to open the token, it counts as elevated and keeps its name.
pub fn process_facts(pid: u32) -> (String, bool) {
    // SAFETY: the handle is closed below; the name buffer outlives the call that fills it.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (String::new(), true);
        };
        let mut buffer = vec![0u16; 1024];
        let mut len = buffer.len() as u32;
        let name = match QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        ) {
            Ok(()) => executable_name(&String::from_utf16_lossy(&buffer[..len as usize])),
            Err(_) => String::new(),
        };
        let elevated = match integrity_level(process) {
            Some(level) => level > own_integrity_level(),
            None => true,
        };
        let _ = CloseHandle(process);
        (name, elevated)
    }
}

/// The file name of an executable's path, in lower case: `C:\Windows\notepad.EXE` gives
/// `notepad.exe`.
pub fn executable_name(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

/// Redpen's own integrity level, read once. Medium (`0x2000`) if it cannot be read.
fn own_integrity_level() -> u32 {
    static LEVEL: OnceLock<u32> = OnceLock::new();
    // SAFETY: the pseudo-handle of the current process needs no closing.
    *LEVEL.get_or_init(|| integrity_level(unsafe { GetCurrentProcess() }).unwrap_or(0x2000))
}

/// The mandatory integrity level of a process's token (the last sub-authority of its label's
/// SID: `0x2000` medium, `0x3000` high), or `None` when Windows refuses to say.
fn integrity_level(process: HANDLE) -> Option<u32> {
    // SAFETY: the token is closed below; the buffer is 8-byte aligned and as long as Windows
    // asked for, and the SID it holds is read while the buffer lives.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut needed);
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
        let result = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr().cast()),
            (buffer.len() * 8) as u32,
            &mut needed,
        );
        let level = result.ok().and_then(|()| {
            let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
            let sid = label.Label.Sid;
            let count = *GetSidSubAuthorityCount(sid);
            if count == 0 {
                return None;
            }
            Some(*GetSidSubAuthority(sid, u32::from(count) - 1))
        });
        let _ = CloseHandle(token);
        level
    }
}

/// The focus, the caret and the rest of a thread's input state.
fn gui_thread_info(thread: u32) -> Option<GUITHREADINFO> {
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is sized and outlives the call.
    unsafe { GetGUIThreadInfo(thread, &mut info) }.ok()?;
    Some(info)
}

/// The system caret on the screen, when the thread shows one.
fn caret_rect(info: &GUITHREADINFO) -> Option<Rect> {
    if info.hwndCaret.is_invalid() {
        return None;
    }
    let rc = info.rcCaret;
    let mut top_left = POINT {
        x: rc.left,
        y: rc.top,
    };
    // SAFETY: `top_left` outlives the call; a stale handle only makes it fail.
    if !unsafe { ClientToScreen(info.hwndCaret, &mut top_left) }.as_bool() {
        return None;
    }
    Some(Rect {
        x: top_left.x,
        y: top_left.y,
        width: rc.right - rc.left,
        height: rc.bottom - rc.top,
    })
}

/// Whether `hwnd` is a classic edit box with the `ES_PASSWORD` style.
fn is_password_edit(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    let mut class = [0u16; 32];
    // SAFETY: the buffer outlives the call, which writes at most its length.
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    let name = String::from_utf16_lossy(&class[..len.max(0) as usize]);
    if !name.eq_ignore_ascii_case("Edit") {
        return false;
    }
    // SAFETY: reads a window's style.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    style & ES_PASSWORD as isize != 0
}

/// The mouse pointer on the screen; the origin if Windows will not say.
fn cursor() -> Point {
    let mut p = POINT::default();
    // SAFETY: `p` outlives the call.
    match unsafe { GetCursorPos(&mut p) } {
        Ok(()) => Point { x: p.x, y: p.y },
        Err(_) => Point::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_names_are_lower_case_file_names() {
        assert_eq!(
            executable_name(r"C:\Program Files\1Password\app\8\1Password.exe"),
            "1password.exe"
        );
        assert_eq!(
            executable_name(r"C:\Program Files\Proton\Pass\Proton Pass.exe"),
            "proton pass.exe"
        );
        assert_eq!(executable_name("notepad.EXE"), "notepad.exe");
        assert_eq!(executable_name(""), "");
    }

    #[test]
    fn this_process_is_not_above_itself() {
        let (name, elevated) = process_facts(std::process::id());
        assert!(name.ends_with(".exe"), "{name}");
        assert!(!elevated);
    }

    #[test]
    fn a_process_that_cannot_be_opened_counts_as_elevated() {
        // Process id 0 is the idle process, which no one may open.
        assert_eq!(process_facts(0), (String::new(), true));
    }

    #[test]
    fn a_closed_window_is_gone() {
        let w = WindowRef { id: 0, pid: 1 };
        assert_eq!(inspect(&w), Ok(None));
    }
}
