//! UI Automation: whether the focused control is a password field (`CurrentIsPassword`).
//!
//! Each question runs on a thread of its own in the multithreaded apartment, as UI Automation
//! asks of clients, so it never waits on the caller's own windows (Redpen's popup) and never
//! changes the caller's COM state. Connections and calls give up after [`UIA_TIMEOUT_MS`]. When
//! UI Automation cannot answer, the capture stops with [`CaptureError::Os`] rather than guess.
//!
//! **Asked twice.** Chromium (Chrome, Edge, and Electron apps such as Slack, Teams and VS Code)
//! builds its accessibility tree only once a UI Automation client asks. Measured during
//! implementation on a fresh Edge 157 (three runs) and Chrome 156 (one run) process showing a
//! page whose password field had the focus: the first query answered with the browser's frame
//! (class `View` in Edge, `Chrome_WidgetWin_1` in Chrome) and no password flag every time, and a
//! second query straight after answered with the password field every time. So whenever the
//! first answer is not a password field, the question is asked once more on the same client
//! (about 4 ms) and the second answer counts.

use ::windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use ::windows::Win32::UI::Accessibility::{CUIAutomation8, IUIAutomation2};
use redpen_core::platform::CaptureError;

use super::{hresult_error, os_error};

/// How long UI Automation may wait for an app to answer, in milliseconds.
pub const UIA_TIMEOUT_MS: u32 = 1_000;

/// The focused control, as UI Automation describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedElement {
    pub process_id: u32,
    pub is_password: bool,
    pub class_name: String,
}

/// The control that has the keyboard focus now, wherever it is: one query.
pub fn focused_element() -> Result<FocusedElement, CaptureError> {
    on_uia_thread(focused)
}

/// Whether the focused control is a password field: one query, and a second one when the first
/// does not say so (see the module's notes).
pub fn focused_is_password() -> Result<bool, CaptureError> {
    on_uia_thread(|automation| {
        if focused(automation)?.is_password {
            return Ok(true);
        }
        Ok(focused(automation)?.is_password)
    })
}

/// Runs `ask` with a UI Automation client on a new thread in the multithreaded apartment.
fn on_uia_thread<T: Send>(
    ask: impl FnOnce(&IUIAutomation2) -> Result<T, CaptureError> + Send,
) -> Result<T, CaptureError> {
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                // SAFETY: COM is initialised on this new thread and uninitialised by the guard
                // after the client has been released (it is declared after the guard, so it is
                // dropped first).
                unsafe {
                    let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
                    if hr.is_err() {
                        return Err(hresult_error(hr));
                    }
                    let _com = ComGuard;
                    let automation: IUIAutomation2 =
                        CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                            .map_err(|e| os_error(&e))?;
                    automation
                        .SetConnectionTimeout(UIA_TIMEOUT_MS)
                        .map_err(|e| os_error(&e))?;
                    automation
                        .SetTransactionTimeout(UIA_TIMEOUT_MS)
                        .map_err(|e| os_error(&e))?;
                    ask(&automation)
                }
            })
            .join()
            // E_UNEXPECTED: the thread panicked.
            .unwrap_or_else(|_| Err(CaptureError::os("0x8000FFFF")))
    })
}

fn focused(automation: &IUIAutomation2) -> Result<FocusedElement, CaptureError> {
    // SAFETY: COM calls on a live client, from the thread that created it.
    unsafe {
        let element = automation.GetFocusedElement().map_err(|e| os_error(&e))?;
        let is_password = element
            .CurrentIsPassword()
            .map_err(|e| os_error(&e))?
            .as_bool();
        let process_id = element.CurrentProcessId().map_err(|e| os_error(&e))? as u32;
        let class_name = element
            .CurrentClassName()
            .map(|name| name.to_string())
            .unwrap_or_default();
        Ok(FocusedElement {
            process_id,
            is_password,
            class_name,
        })
    }
}

/// Uninitialises COM on this thread when dropped.
struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: pairs with the successful CoInitializeEx on this thread.
        unsafe { CoUninitialize() };
    }
}
