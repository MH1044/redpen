//! Keys on Windows: whether a modifier is down (`GetAsyncKeyState`), and the synthetic Ctrl+C,
//! Ctrl+V and Ctrl+Z (`SendInput`). The flows wait for the modifiers to be up before any of
//! these, so the user's own keys never mix with Redpen's.

use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_C, VK_CONTROL,
    VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V, VK_Z,
};
use redpen_core::platform::CaptureError;

use super::last_error;

/// The keys Redpen sends, always with Ctrl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Copy.
    C,
    /// Paste.
    V,
    /// Undo.
    Z,
}

impl Key {
    fn virtual_key(self) -> VIRTUAL_KEY {
        match self {
            Key::C => VK_C,
            Key::V => VK_V,
            Key::Z => VK_Z,
        }
    }
}

/// The modifiers the key wait watches: Ctrl, Alt, Shift and both Windows keys.
pub const MODIFIERS: [VIRTUAL_KEY; 5] = [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN];

/// Whether any of [`MODIFIERS`] is down now.
pub fn modifiers_down() -> bool {
    MODIFIERS.iter().any(|vk| {
        // SAFETY: reads the asynchronous key state; no pointers.
        let state = unsafe { GetAsyncKeyState(i32::from(vk.0)) };
        (state as u16) & 0x8000 != 0
    })
}

/// Sends Ctrl+`key` to the window in front: Ctrl down, the key down, the key up, Ctrl up.
pub fn send_ctrl(key: Key) -> Result<(), CaptureError> {
    let vk = key.virtual_key();
    let inputs = [
        keyboard(VK_CONTROL, false),
        keyboard(vk, false),
        keyboard(vk, true),
        keyboard(VK_CONTROL, true),
    ];
    if send(&inputs) == inputs.len() {
        return Ok(());
    }
    let error = last_error();
    // A partial send could leave Ctrl down: release both keys whatever happened.
    send(&[keyboard(vk, true), keyboard(VK_CONTROL, true)]);
    Err(error)
}

fn send(inputs: &[INPUT]) -> usize {
    // SAFETY: the slice holds fully initialised keyboard inputs of the size passed.
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    sent as usize
}

/// One keyboard event, with the key's scan code for the apps that read it.
fn keyboard(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    // SAFETY: a pure table lookup.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan as u16,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_is_ctrl_down_key_down_key_up_ctrl_up() {
        let down = keyboard(VK_C, false);
        let up = keyboard(VK_C, true);
        // SAFETY: both were built as keyboard inputs.
        unsafe {
            assert_eq!(down.r#type, INPUT_KEYBOARD);
            assert_eq!(down.Anonymous.ki.wVk, VK_C);
            assert_eq!(down.Anonymous.ki.dwFlags, KEYBD_EVENT_FLAGS(0));
            assert_eq!(up.Anonymous.ki.dwFlags, KEYEVENTF_KEYUP);
            // Every keyboard layout has a C key, so the scan code is found.
            assert_ne!(down.Anonymous.ki.wScan, 0);
        }
        assert_eq!(Key::V.virtual_key(), VK_V);
        assert_eq!(Key::Z.virtual_key(), VK_Z);
    }

    #[test]
    fn the_modifiers_are_ctrl_alt_shift_and_windows() {
        let codes: Vec<u16> = MODIFIERS.iter().map(|vk| vk.0).collect();
        assert_eq!(codes, [0x11, 0x12, 0x10, 0x5B, 0x5C]);
    }
}
