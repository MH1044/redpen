//! [`FakeCapture`]: a [`Capture`] for tests, scripted call by call, that records every call in
//! order (docs/modules/05-capture-and-replace-windows.md, Design notes).
//!
//! Without a script it behaves like a small desktop: windows that exist or not and can be
//! brought to the front or not, a clipboard holding text with a sequence number, and a
//! selection that Ctrl+C copies. `read_selection` saves the clipboard, copies, and restores it, so
//! a test can see that the clipboard holds what it held before. A scripted answer
//! (`script_*`) replaces the model's for one call, in the order given.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use super::capture::{Capture, CaptureError, Foreground, WindowRef};

/// One call to the fake, with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureCall {
    Foreground,
    Inspect(WindowRef),
    ModifiersDown,
    ReadSelection {
        timeout: Duration,
    },
    Focus(WindowRef),
    Paste {
        text: String,
        restore_delay: Duration,
    },
    SendUndo,
    SetClipboardText(String),
}

impl CaptureCall {
    /// The trait method's name: `foreground`, `inspect`, `modifiers_down`, `read_selection`,
    /// `focus`, `paste`, `send_undo` or `set_clipboard_text`.
    pub fn name(&self) -> &'static str {
        match self {
            CaptureCall::Foreground => "foreground",
            CaptureCall::Inspect(_) => "inspect",
            CaptureCall::ModifiersDown => "modifiers_down",
            CaptureCall::ReadSelection { .. } => "read_selection",
            CaptureCall::Focus(_) => "focus",
            CaptureCall::Paste { .. } => "paste",
            CaptureCall::SendUndo => "send_undo",
            CaptureCall::SetClipboardText(_) => "set_clipboard_text",
        }
    }
}

/// A window of the fake desktop.
#[derive(Debug, Clone)]
struct FakeWindow {
    info: Foreground,
    focusable: bool,
}

#[derive(Debug, Default)]
struct State {
    calls: Vec<CaptureCall>,
    windows: BTreeMap<u64, FakeWindow>,
    front: Option<u64>,
    clipboard: Option<String>,
    sequence: u32,
    selection: Option<String>,
    modifiers: VecDeque<bool>,
    typed: Vec<(WindowRef, String)>,
    undone: Vec<WindowRef>,
    foreground_script: VecDeque<Result<Foreground, CaptureError>>,
    inspect_script: VecDeque<Result<Option<Foreground>, CaptureError>>,
    read_script: VecDeque<Result<String, CaptureError>>,
    focus_script: VecDeque<Result<bool, CaptureError>>,
    paste_script: VecDeque<Result<(), CaptureError>>,
    undo_script: VecDeque<Result<(), CaptureError>>,
    clipboard_script: VecDeque<Result<(), CaptureError>>,
}

/// A scripted, recording [`Capture`].
///
/// [`FakeCapture::new`] starts with one window in front, [`FakeCapture::default_window`]
/// (`notepad.exe`, nothing refused), an empty clipboard and no selection, so `read_selection`
/// gives [`CaptureError::NothingCopied`] until [`FakeCapture::set_selection`].
#[derive(Debug)]
pub struct FakeCapture {
    state: Mutex<State>,
}

impl Default for FakeCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCapture {
    /// The window [`FakeCapture::new`] puts in front: id 1, process id 1000, `notepad.exe`.
    pub fn default_window() -> Foreground {
        Foreground {
            window: WindowRef { id: 1, pid: 1000 },
            process: "notepad.exe".into(),
            ..Foreground::default()
        }
    }

    pub fn new() -> Self {
        let fake = Self {
            state: Mutex::new(State::default()),
        };
        fake.set_foreground(Self::default_window());
        fake
    }

    /// A fake with `fg` as the only window, in front.
    pub fn with_foreground(fg: Foreground) -> Self {
        let fake = Self {
            state: Mutex::new(State::default()),
        };
        fake.set_foreground(fg);
        fake
    }

    /// Sets what Ctrl+C copies; `None` makes the copy change nothing.
    pub fn with_selection(self, text: impl Into<String>) -> Self {
        self.set_selection(Some(text.into()));
        self
    }

    /// Sets the clipboard's text.
    pub fn with_clipboard(self, text: impl Into<String>) -> Self {
        self.set_clipboard(Some(text.into()));
        self
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("fake capture state is not poisoned")
    }

    /// Adds `fg`'s window (or replaces it) and puts it in front.
    pub fn set_foreground(&self, fg: Foreground) {
        let mut s = self.state();
        let id = fg.window.id;
        s.windows.insert(
            id,
            FakeWindow {
                info: fg,
                focusable: true,
            },
        );
        s.front = Some(id);
    }

    /// Adds a window (or replaces what is known about it) without changing which is in front.
    pub fn add_window(&self, fg: Foreground) {
        let mut s = self.state();
        let focusable = s.windows.get(&fg.window.id).is_none_or(|w| w.focusable);
        s.windows.insert(
            fg.window.id,
            FakeWindow {
                info: fg,
                focusable,
            },
        );
    }

    /// Changes what is known about an existing window (its focused control became a password
    /// field, for example). Does nothing when the window is gone.
    pub fn update_window(&self, w: &WindowRef, change: impl FnOnce(&mut Foreground)) {
        if let Some(window) = self.state().windows.get_mut(&w.id) {
            change(&mut window.info);
        }
    }

    /// Closes a window. When it was in front, no window is in front until another is set.
    pub fn close_window(&self, w: &WindowRef) {
        let mut s = self.state();
        s.windows.remove(&w.id);
        if s.front == Some(w.id) {
            s.front = None;
        }
    }

    /// Whether `focus` can bring the window to the front.
    pub fn set_focusable(&self, w: &WindowRef, focusable: bool) {
        if let Some(window) = self.state().windows.get_mut(&w.id) {
            window.focusable = focusable;
        }
    }

    /// Sets what Ctrl+C copies; `None` makes the copy change nothing.
    pub fn set_selection(&self, text: Option<String>) {
        self.state().selection = text;
    }

    /// Sets the clipboard's text, as another app would (the sequence number moves).
    pub fn set_clipboard(&self, text: Option<String>) {
        let mut s = self.state();
        s.clipboard = text;
        s.sequence += 1;
    }

    /// The clipboard's text now.
    pub fn clipboard(&self) -> Option<String> {
        self.state().clipboard.clone()
    }

    /// The clipboard's sequence number, moved by every write.
    pub fn sequence(&self) -> u32 {
        self.state().sequence
    }

    /// The window in front now, if any.
    pub fn front(&self) -> Option<WindowRef> {
        let s = self.state();
        s.front
            .and_then(|id| s.windows.get(&id))
            .map(|w| w.info.window)
    }

    /// Keeps a modifier key down for the next `polls` calls of `modifiers_down`.
    pub fn hold_modifiers(&self, polls: usize) {
        self.state()
            .modifiers
            .extend(std::iter::repeat_n(true, polls));
    }

    /// The next answers of `modifiers_down`, in order; after them it answers false.
    pub fn script_modifiers(&self, answers: impl IntoIterator<Item = bool>) {
        self.state().modifiers.extend(answers);
    }

    pub fn script_foreground(&self, answer: Result<Foreground, CaptureError>) {
        self.state().foreground_script.push_back(answer);
    }

    pub fn script_inspect(&self, answer: Result<Option<Foreground>, CaptureError>) {
        self.state().inspect_script.push_back(answer);
    }

    pub fn script_read_selection(&self, answer: Result<String, CaptureError>) {
        self.state().read_script.push_back(answer);
    }

    pub fn script_focus(&self, answer: Result<bool, CaptureError>) {
        self.state().focus_script.push_back(answer);
    }

    pub fn script_paste(&self, answer: Result<(), CaptureError>) {
        self.state().paste_script.push_back(answer);
    }

    pub fn script_send_undo(&self, answer: Result<(), CaptureError>) {
        self.state().undo_script.push_back(answer);
    }

    pub fn script_set_clipboard_text(&self, answer: Result<(), CaptureError>) {
        self.state().clipboard_script.push_back(answer);
    }

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<CaptureCall> {
        self.state().calls.clone()
    }

    /// The names of every call so far, in order.
    pub fn call_names(&self) -> Vec<&'static str> {
        self.state().calls.iter().map(CaptureCall::name).collect()
    }

    /// How many calls of the method named `name` (as [`CaptureCall::name`] spells it).
    pub fn count(&self, name: &str) -> usize {
        self.state()
            .calls
            .iter()
            .filter(|c| c.name() == name)
            .count()
    }

    /// The text each unscripted `paste` typed, with the window that was in front.
    pub fn typed(&self) -> Vec<(WindowRef, String)> {
        self.state().typed.clone()
    }

    /// The window in front at each unscripted `send_undo`.
    pub fn undone(&self) -> Vec<WindowRef> {
        self.state().undone.clone()
    }

    /// Forgets the calls recorded so far (the model and the scripts stay).
    pub fn clear_calls(&self) {
        self.state().calls.clear();
    }
}

impl State {
    fn front_info(&self) -> Option<Foreground> {
        self.front
            .and_then(|id| self.windows.get(&id))
            .map(|w| w.info.clone())
    }

    fn window(&self, w: &WindowRef) -> Option<&FakeWindow> {
        self.windows
            .get(&w.id)
            .filter(|x| x.info.window.pid == w.pid)
    }
}

/// What `foreground` answers when no window is in front: Win32's "invalid window handle".
const NO_WINDOW: u32 = 1400;

impl Capture for FakeCapture {
    fn foreground(&self) -> Result<Foreground, CaptureError> {
        let mut s = self.state();
        s.calls.push(CaptureCall::Foreground);
        if let Some(answer) = s.foreground_script.pop_front() {
            return answer;
        }
        s.front_info().ok_or_else(|| CaptureError::os(NO_WINDOW))
    }

    fn inspect(&self, w: &WindowRef) -> Result<Option<Foreground>, CaptureError> {
        let mut s = self.state();
        s.calls.push(CaptureCall::Inspect(*w));
        if let Some(answer) = s.inspect_script.pop_front() {
            return answer;
        }
        Ok(s.window(w).map(|x| x.info.clone()))
    }

    fn modifiers_down(&self) -> bool {
        let mut s = self.state();
        s.calls.push(CaptureCall::ModifiersDown);
        s.modifiers.pop_front().unwrap_or(false)
    }

    fn read_selection(&self, timeout: Duration) -> Result<String, CaptureError> {
        let mut s = self.state();
        s.calls.push(CaptureCall::ReadSelection { timeout });
        if let Some(answer) = s.read_script.pop_front() {
            return answer;
        }
        // Save, copy, read, restore: what the Windows implementation does.
        let saved = s.clipboard.clone();
        let before = s.sequence;
        if let Some(text) = s.selection.clone() {
            s.clipboard = Some(text);
            s.sequence += 1;
        }
        if s.sequence == before {
            // Nothing changed within the timeout, so there is nothing to put back.
            return Err(CaptureError::NothingCopied);
        }
        let copied = s.clipboard.take().unwrap_or_default();
        s.clipboard = saved;
        s.sequence += 1;
        if copied.is_empty() {
            Err(CaptureError::NothingCopied)
        } else {
            Ok(copied)
        }
    }

    fn focus(&self, w: &WindowRef) -> Result<bool, CaptureError> {
        let mut s = self.state();
        s.calls.push(CaptureCall::Focus(*w));
        if let Some(answer) = s.focus_script.pop_front() {
            return answer;
        }
        match s.window(w).map(|x| x.focusable) {
            None => Err(CaptureError::WindowGone),
            Some(false) => Ok(s.front == Some(w.id)),
            Some(true) => {
                s.front = Some(w.id);
                Ok(true)
            }
        }
    }

    fn paste(&self, text: &str, restore_delay: Duration) -> Result<(), CaptureError> {
        let mut s = self.state();
        s.calls.push(CaptureCall::Paste {
            text: text.to_string(),
            restore_delay,
        });
        if let Some(answer) = s.paste_script.pop_front() {
            return answer;
        }
        let front = s
            .front_info()
            .ok_or_else(|| CaptureError::os(NO_WINDOW))?
            .window;
        s.typed.push((front, text.to_string()));
        // The temporary entry, then the restored one.
        s.sequence += 2;
        Ok(())
    }

    fn send_undo(&self) -> Result<(), CaptureError> {
        let mut s = self.state();
        s.calls.push(CaptureCall::SendUndo);
        if let Some(answer) = s.undo_script.pop_front() {
            return answer;
        }
        let front = s
            .front_info()
            .ok_or_else(|| CaptureError::os(NO_WINDOW))?
            .window;
        s.undone.push(front);
        Ok(())
    }

    fn set_clipboard_text(&self, text: &str) -> Result<(), CaptureError> {
        let mut s = self.state();
        s.calls
            .push(CaptureCall::SetClipboardText(text.to_string()));
        if let Some(answer) = s.clipboard_script.pop_front() {
            return answer;
        }
        s.clipboard = Some(text.to_string());
        s.sequence += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64, process: &str) -> Foreground {
        Foreground {
            window: WindowRef {
                id,
                pid: id as u32 * 10,
            },
            process: process.into(),
            ..Foreground::default()
        }
    }

    #[test]
    fn records_every_call_in_order() {
        let fake = FakeCapture::new().with_selection("wrold");
        fake.foreground().unwrap();
        fake.modifiers_down();
        fake.read_selection(Duration::from_millis(400)).unwrap();
        assert_eq!(
            fake.call_names(),
            ["foreground", "modifiers_down", "read_selection"]
        );
        assert_eq!(
            fake.calls()[2],
            CaptureCall::ReadSelection {
                timeout: Duration::from_millis(400)
            }
        );
        assert_eq!(fake.count("read_selection"), 1);
        fake.clear_calls();
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn read_selection_restores_the_clipboard() {
        let fake = FakeCapture::new()
            .with_clipboard("keep me")
            .with_selection("wrold");
        let before = fake.sequence();
        assert_eq!(fake.read_selection(Duration::ZERO).unwrap(), "wrold");
        assert_eq!(fake.clipboard().as_deref(), Some("keep me"));
        assert!(fake.sequence() > before);
    }

    #[test]
    fn a_copy_that_changes_nothing_is_nothing_copied() {
        let fake = FakeCapture::new().with_clipboard("keep me");
        let before = fake.sequence();
        assert_eq!(
            fake.read_selection(Duration::from_millis(400)),
            Err(CaptureError::NothingCopied)
        );
        assert_eq!(fake.clipboard().as_deref(), Some("keep me"));
        assert_eq!(fake.sequence(), before);
        let empty = FakeCapture::new().with_selection("");
        assert_eq!(
            empty.read_selection(Duration::ZERO),
            Err(CaptureError::NothingCopied)
        );
    }

    #[test]
    fn scripts_answer_one_call_each_in_order() {
        let fake = FakeCapture::new();
        fake.script_read_selection(Err(CaptureError::ClipboardBusy));
        fake.script_read_selection(Ok("one".into()));
        assert_eq!(
            fake.read_selection(Duration::ZERO),
            Err(CaptureError::ClipboardBusy)
        );
        assert_eq!(fake.read_selection(Duration::ZERO).unwrap(), "one");
        assert_eq!(
            fake.read_selection(Duration::ZERO),
            Err(CaptureError::NothingCopied)
        );
        fake.script_modifiers([true, false, true]);
        assert!(fake.modifiers_down());
        assert!(!fake.modifiers_down());
        assert!(fake.modifiers_down());
        assert!(!fake.modifiers_down());
        fake.hold_modifiers(2);
        assert!(fake.modifiers_down() && fake.modifiers_down());
        assert!(!fake.modifiers_down());
    }

    #[test]
    fn windows_open_close_and_take_focus() {
        let fake = FakeCapture::with_foreground(window(1, "notepad.exe"));
        let other = window(2, "winword.exe");
        fake.add_window(other.clone());
        assert_eq!(fake.front(), Some(window(1, "").window));
        assert_eq!(fake.focus(&other.window), Ok(true));
        assert_eq!(fake.foreground().unwrap().process, "winword.exe");
        fake.set_focusable(&window(1, "").window, false);
        assert_eq!(fake.focus(&window(1, "").window), Ok(false));
        fake.close_window(&other.window);
        assert_eq!(fake.inspect(&other.window), Ok(None));
        assert_eq!(fake.focus(&other.window), Err(CaptureError::WindowGone));
        assert_eq!(fake.foreground(), Err(CaptureError::os(1400)));
        // A handle reused by another process is a different window.
        let reused = WindowRef { id: 1, pid: 99 };
        assert_eq!(fake.inspect(&reused), Ok(None));
    }

    #[test]
    fn paste_and_undo_go_to_the_window_in_front() {
        let fake = FakeCapture::new().with_clipboard("keep me");
        fake.update_window(&FakeCapture::default_window().window, |w| {
            w.caret = None;
        });
        fake.paste("world", Duration::from_millis(250)).unwrap();
        fake.send_undo().unwrap();
        let front = FakeCapture::default_window().window;
        assert_eq!(fake.typed(), vec![(front, "world".to_string())]);
        assert_eq!(fake.undone(), vec![front]);
        assert_eq!(fake.clipboard().as_deref(), Some("keep me"));
        fake.set_clipboard_text("copied").unwrap();
        assert_eq!(fake.clipboard().as_deref(), Some("copied"));
    }
}
