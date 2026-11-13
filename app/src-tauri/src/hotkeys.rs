//! The check shortcut (docs/modules/06-popup-and-settings-ui.md): `hotkeys.check` in
//! `settings.json`, Ctrl+Alt+R by default, registered with the operating system through
//! `tauri-plugin-global-shortcut` from Rust only (the web view gets no permission for it).
//!
//! The rules live in [`Hotkeys`], written against two traits so tests run them with fakes:
//!
//! - [`HotkeyRegistrar`] registers and unregisters a combination; the production one is
//!   [`PluginRegistrar`]. A refused registration (another app holds the combination) is
//!   [`HotkeyError::InUse`], naming it.
//! - [`KeyboardLayout`] says whether a combination types a character on the active keyboard
//!   layout: on many European layouts Ctrl+Alt is AltGr, so Polish Ctrl+Alt+C types `ć` and
//!   German Ctrl+Alt+E types `€` (docs/ARCHITECTURE.md §10). Such a combination is
//!   [`HotkeyError::AltGr`]. On Windows, [`WindowsLayout`] asks `ToUnicodeEx` with Ctrl and Alt
//!   down; elsewhere Ctrl+Alt types nothing ([`NoAltGr`]).
//!
//! [`Hotkeys::change`] registers the new combination before letting go of the old one, so a
//! refused change leaves the old one working. [`Hotkeys::pause`] (Pause Redpen in the tray)
//! unregisters every hotkey and [`Hotkeys::resume`] registers the same set again.

use std::fmt;
use std::sync::Mutex;

use serde::Serialize;

/// The default check shortcut.
pub const DEFAULT_CHECK: &str = "Ctrl+Alt+R";

/// Registers combinations such as `Ctrl+Alt+R` with the operating system.
pub trait HotkeyRegistrar: Send + Sync {
    /// Registers `hotkey`; `Err` with the reason when the system refuses it (usually because
    /// another app holds it).
    fn register(&self, hotkey: &str) -> Result<(), String>;
    /// Lets go of `hotkey`. Letting go of one that is not registered does nothing.
    fn unregister(&self, hotkey: &str);
    /// Every combination registered now.
    fn registered(&self) -> Vec<String>;
}

/// The active keyboard layout.
pub trait KeyboardLayout: Send + Sync {
    /// The character `hotkey` types on this layout, if it types one.
    fn types_character(&self, hotkey: &str) -> Option<char>;
}

/// Why a shortcut was refused. `Display` is the message Settings shows under the shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    /// The system refused the registration: another app uses the combination.
    InUse { hotkey: String },
    /// The combination types `character` on the active keyboard layout (AltGr).
    AltGr { hotkey: String, character: char },
    /// Not a combination Redpen can register.
    Invalid { hotkey: String },
}

impl HotkeyError {
    /// `HotkeyInUse`, or `HotkeyInvalid` for a combination Redpen will not use (a malformed one,
    /// or one that types a character).
    pub fn kind(&self) -> &'static str {
        match self {
            HotkeyError::InUse { .. } => "HotkeyInUse",
            HotkeyError::AltGr { .. } | HotkeyError::Invalid { .. } => "HotkeyInvalid",
        }
    }

    /// The combination that was refused.
    pub fn hotkey(&self) -> &str {
        match self {
            HotkeyError::InUse { hotkey }
            | HotkeyError::AltGr { hotkey, .. }
            | HotkeyError::Invalid { hotkey } => hotkey,
        }
    }

    /// `{"kind", "message", "hotkey"}`, plus `character` for AltGr.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("an error serialises")
    }
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HotkeyError::InUse { hotkey } => write!(
                f,
                "{hotkey} is already used by another app. Choose another shortcut."
            ),
            HotkeyError::AltGr { .. } => f.write_str(
                "That shortcut types a character on your keyboard layout. Choose another.",
            ),
            HotkeyError::Invalid { .. } => f.write_str(
                "Use Ctrl, Alt or the Windows key, with Shift if you like, and one letter, digit or F1 to F24.",
            ),
        }
    }
}

impl std::error::Error for HotkeyError {}

impl Serialize for HotkeyError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("kind", self.kind())?;
        map.serialize_entry("message", &self.to_string())?;
        map.serialize_entry("hotkey", self.hotkey())?;
        if let HotkeyError::AltGr { character, .. } = self {
            map.serialize_entry("character", &character.to_string())?;
        }
        map.end()
    }
}

/// The key of a combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// `A` to `Z`.
    Letter(char),
    /// `0` to `9`.
    Digit(char),
    /// `F1` to `F24`.
    F(u8),
    Space,
}

impl Key {
    fn parse(token: &str) -> Option<Key> {
        let upper = token.to_ascii_uppercase();
        let mut chars = upper.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_uppercase() => return Some(Key::Letter(c)),
            (Some(c), None) if c.is_ascii_digit() => return Some(Key::Digit(c)),
            _ => {}
        }
        if upper == "SPACE" {
            return Some(Key::Space);
        }
        let n: u8 = upper.strip_prefix('F')?.parse().ok()?;
        (1..=24).contains(&n).then_some(Key::F(n))
    }

    fn name(self) -> String {
        match self {
            Key::Letter(c) | Key::Digit(c) => c.to_string(),
            Key::F(n) => format!("F{n}"),
            Key::Space => "Space".into(),
        }
    }

    /// The Windows virtual-key code.
    pub fn virtual_key(self) -> u16 {
        match self {
            Key::Letter(c) | Key::Digit(c) => c as u16,
            Key::F(n) => 0x70 + u16::from(n) - 1,
            Key::Space => 0x20,
        }
    }
}

/// A parsed combination: modifiers and one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

impl Combo {
    /// Parses `Ctrl+Alt+R` and the like: modifiers (`Ctrl` or `Control`, `Alt`, `Shift`, `Win`,
    /// `Super` or `Meta`) in any order and case, then one key. At least one of Ctrl, Alt and Win,
    /// so the combination never types on its own.
    pub fn parse(hotkey: &str) -> Option<Combo> {
        let tokens: Vec<&str> = hotkey.split('+').map(str::trim).collect();
        let (key, modifiers) = tokens.split_last()?;
        let mut combo = Combo {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: Key::parse(key)?,
        };
        for m in modifiers {
            let flag = match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut combo.ctrl,
                "alt" => &mut combo.alt,
                "shift" => &mut combo.shift,
                "win" | "super" | "meta" => &mut combo.win,
                _ => return None,
            };
            if *flag {
                return None;
            }
            *flag = true;
        }
        (combo.ctrl || combo.alt || combo.win).then_some(combo)
    }

    /// The canonical spelling: `Ctrl+Alt+Shift+Win+<key>`, modifiers in that order.
    pub fn name(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.win, "Win"),
        ] {
            if on {
                parts.push(name.into());
            }
        }
        parts.push(self.key.name());
        parts.join("+")
    }
}

/// `hotkey` in its canonical spelling, or [`HotkeyError::Invalid`].
pub fn normalise(hotkey: &str) -> Result<String, HotkeyError> {
    Combo::parse(hotkey)
        .map(|c| c.name())
        .ok_or_else(|| HotkeyError::Invalid {
            hotkey: hotkey.trim().to_string(),
        })
}

/// Redpen's hotkeys: the check shortcut, and whether Redpen is paused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkeys {
    check: String,
    paused: bool,
}

impl Hotkeys {
    /// The check shortcut `check` (in its canonical spelling when it parses), not registered
    /// yet.
    pub fn new(check: &str) -> Self {
        Self {
            check: normalise(check).unwrap_or_else(|_| check.trim().to_string()),
            paused: false,
        }
    }

    /// The check shortcut.
    pub fn check(&self) -> &str {
        &self.check
    }

    /// Every hotkey Redpen registers while it is not paused.
    pub fn all(&self) -> Vec<String> {
        vec![self.check.clone()]
    }

    /// Whether Pause Redpen is on.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// At start: registers every hotkey, unless `paused` (the setting `ui.paused`).
    pub fn start(
        &mut self,
        registrar: &dyn HotkeyRegistrar,
        paused: bool,
    ) -> Result<(), HotkeyError> {
        self.paused = paused;
        if paused {
            return Ok(());
        }
        self.register_all(registrar)
    }

    fn register_all(&self, registrar: &dyn HotkeyRegistrar) -> Result<(), HotkeyError> {
        for hotkey in self.all() {
            Combo::parse(&hotkey).ok_or_else(|| HotkeyError::Invalid {
                hotkey: hotkey.clone(),
            })?;
            if registrar.registered().contains(&hotkey) {
                continue;
            }
            if registrar.register(&hotkey).is_err() {
                tracing::info!("a hotkey registration was refused");
                return Err(HotkeyError::InUse { hotkey });
            }
        }
        Ok(())
    }

    /// Changes the check shortcut to `new`. Refused, with the old shortcut still registered,
    /// when `new` is not a combination Redpen can use, types a character on `layout`, or cannot
    /// be registered. While paused the new shortcut is only remembered, and registered on
    /// resume.
    pub fn change(
        &mut self,
        new: &str,
        registrar: &dyn HotkeyRegistrar,
        layout: &dyn KeyboardLayout,
    ) -> Result<(), HotkeyError> {
        let new = normalise(new)?;
        if let Some(character) = layout.types_character(&new).filter(|c| !c.is_control()) {
            return Err(HotkeyError::AltGr {
                hotkey: new,
                character,
            });
        }
        if new == self.check {
            // The same shortcut: registered again only when the system refused it before (at
            // start, while another app held it).
            if self.paused || registrar.registered().contains(&new) {
                return Ok(());
            }
            if registrar.register(&new).is_err() {
                tracing::info!("a hotkey registration was refused");
                return Err(HotkeyError::InUse { hotkey: new });
            }
            return Ok(());
        }
        if !self.paused {
            if registrar.register(&new).is_err() {
                tracing::info!("a hotkey registration was refused");
                return Err(HotkeyError::InUse { hotkey: new });
            }
            registrar.unregister(&self.check);
        }
        self.check = new;
        Ok(())
    }

    /// Pause Redpen: unregisters every hotkey.
    pub fn pause(&mut self, registrar: &dyn HotkeyRegistrar) {
        for hotkey in self.all() {
            registrar.unregister(&hotkey);
        }
        self.paused = true;
    }

    /// Resume: registers the same set again.
    pub fn resume(&mut self, registrar: &dyn HotkeyRegistrar) -> Result<(), HotkeyError> {
        self.paused = false;
        self.register_all(registrar)
    }
}

/// A layout on which Ctrl+Alt never types: Linux, where AltGr is a key of its own.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoAltGr;

impl KeyboardLayout for NoAltGr {
    fn types_character(&self, _hotkey: &str) -> Option<char> {
        None
    }
}

/// The active keyboard layout on Windows: `ToUnicodeEx` for the combination's key with Ctrl and
/// Alt (and Shift, when the combination has it) down, on the layout of the window in front,
/// without changing the keyboard state. Only a combination with both Ctrl and Alt can be AltGr.
#[cfg(windows)]
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsLayout;

#[cfg(windows)]
impl KeyboardLayout for WindowsLayout {
    fn types_character(&self, hotkey: &str) -> Option<char> {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardLayout, MAPVK_VK_TO_VSC_EX, MapVirtualKeyExW, ToUnicodeEx, VK_CONTROL,
            VK_LCONTROL, VK_MENU, VK_RMENU, VK_SHIFT,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        let combo = Combo::parse(hotkey)?;
        if !(combo.ctrl && combo.alt) {
            return None;
        }
        let vk = combo.key.virtual_key();
        let mut state = [0u8; 256];
        for key in [VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU] {
            state[usize::from(key.0)] = 0x80;
        }
        if combo.shift {
            state[usize::from(VK_SHIFT.0)] = 0x80;
        }
        let mut buf = [0u16; 8];
        // SAFETY: the calls take a window handle (null when nothing is in front, which gives the
        // calling thread's layout), plain integers and buffers that live for the call.
        let written = unsafe {
            let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let layout = GetKeyboardLayout(thread);
            let scan = MapVirtualKeyExW(u32::from(vk), MAPVK_VK_TO_VSC_EX, Some(layout));
            // Flag 4: leave the keyboard state (dead keys) as it was.
            ToUnicodeEx(u32::from(vk), scan, &state, &mut buf, 4, Some(layout))
        };
        let units = usize::try_from(written.unsigned_abs()).ok()?.min(buf.len());
        if written == 0 {
            return None;
        }
        // A dead key (negative) types a character with the next key press.
        char::decode_utf16(buf[..units.max(1)].iter().copied())
            .next()
            .and_then(Result::ok)
            .filter(|c| !c.is_control())
    }
}

/// The production layout for this operating system.
pub fn system_layout() -> Box<dyn KeyboardLayout> {
    #[cfg(windows)]
    {
        Box::new(WindowsLayout)
    }
    #[cfg(not(windows))]
    {
        Box::new(NoAltGr)
    }
}

/// The production registrar: `tauri-plugin-global-shortcut`, with the set registered through it
/// kept here (the plugin cannot list them).
pub struct PluginRegistrar<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    registered: Mutex<Vec<String>>,
}

impl<R: tauri::Runtime> PluginRegistrar<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self {
            app,
            registered: Mutex::new(Vec::new()),
        }
    }

    fn list(&self) -> std::sync::MutexGuard<'_, Vec<String>> {
        self.registered.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The plugin's shortcut for a combination.
pub fn shortcut(combo: &Combo) -> tauri_plugin_global_shortcut::Shortcut {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};
    let mut mods = Modifiers::empty();
    for (on, m) in [
        (combo.ctrl, Modifiers::CONTROL),
        (combo.alt, Modifiers::ALT),
        (combo.shift, Modifiers::SHIFT),
        (combo.win, Modifiers::SUPER),
    ] {
        if on {
            mods |= m;
        }
    }
    let code = match combo.key {
        Key::Letter(c) => letter_code(c),
        Key::Digit(c) => digit_code(c),
        Key::F(n) => f_code(n),
        Key::Space => Code::Space,
    };
    Shortcut::new(Some(mods), code)
}

fn letter_code(c: char) -> tauri_plugin_global_shortcut::Code {
    use tauri_plugin_global_shortcut::Code::*;
    const LETTERS: [tauri_plugin_global_shortcut::Code; 26] = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ];
    LETTERS[usize::from(c as u8 - b'A')]
}

fn digit_code(c: char) -> tauri_plugin_global_shortcut::Code {
    use tauri_plugin_global_shortcut::Code::*;
    const DIGITS: [tauri_plugin_global_shortcut::Code; 10] = [
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    ];
    DIGITS[usize::from(c as u8 - b'0')]
}

fn f_code(n: u8) -> tauri_plugin_global_shortcut::Code {
    use tauri_plugin_global_shortcut::Code::*;
    const FS: [tauri_plugin_global_shortcut::Code; 24] = [
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20,
        F21, F22, F23, F24,
    ];
    FS[usize::from(n - 1)]
}

impl<R: tauri::Runtime> HotkeyRegistrar for PluginRegistrar<R> {
    fn register(&self, hotkey: &str) -> Result<(), String> {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let combo = Combo::parse(hotkey).ok_or_else(|| "not a combination".to_string())?;
        self.app
            .global_shortcut()
            .register(shortcut(&combo))
            .map_err(|e| e.to_string())?;
        let mut list = self.list();
        if !list.iter().any(|h| h == hotkey) {
            list.push(hotkey.to_string());
        }
        Ok(())
    }

    fn unregister(&self, hotkey: &str) {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        if let Some(combo) = Combo::parse(hotkey)
            && self
                .app
                .global_shortcut()
                .unregister(shortcut(&combo))
                .is_err()
        {
            tracing::warn!("a hotkey could not be unregistered");
        }
        self.list().retain(|h| h != hotkey);
    }

    fn registered(&self) -> Vec<String> {
        self.list().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refuses the combinations it is told to; remembers what is registered.
    #[derive(Default)]
    struct Fake {
        refuse: Vec<String>,
        registered: Mutex<Vec<String>>,
    }

    impl HotkeyRegistrar for Fake {
        fn register(&self, hotkey: &str) -> Result<(), String> {
            if self.refuse.iter().any(|r| r == hotkey) {
                return Err("in use".into());
            }
            self.registered.lock().unwrap().push(hotkey.into());
            Ok(())
        }
        fn unregister(&self, hotkey: &str) {
            self.registered.lock().unwrap().retain(|h| h != hotkey);
        }
        fn registered(&self) -> Vec<String> {
            self.registered.lock().unwrap().clone()
        }
    }

    /// Polish: Ctrl+Alt+C types ć.
    struct Polish;

    impl KeyboardLayout for Polish {
        fn types_character(&self, hotkey: &str) -> Option<char> {
            (hotkey == "Ctrl+Alt+C").then_some('ć')
        }
    }

    #[test]
    fn combinations_parse_and_print_canonically() {
        assert_eq!(normalise("ctrl+alt+r").unwrap(), "Ctrl+Alt+R");
        assert_eq!(normalise(" Alt + Control + k ").unwrap(), "Ctrl+Alt+K");
        assert_eq!(normalise("Shift+Win+F12").unwrap(), "Shift+Win+F12");
        assert_eq!(normalise("Ctrl+Space").unwrap(), "Ctrl+Space");
        assert_eq!(normalise("Ctrl+7").unwrap(), "Ctrl+7");
        for bad in [
            "R",
            "Shift+R",
            "Ctrl+Alt",
            "Ctrl+Ctrl+R",
            "Ctrl+Alt+RR",
            "Hyper+R",
            "Ctrl+F25",
            "",
        ] {
            let e = normalise(bad).unwrap_err();
            assert_eq!(e.kind(), "HotkeyInvalid", "{bad}");
        }
    }

    #[test]
    fn virtual_keys_are_windows_codes() {
        assert_eq!(Key::Letter('R').virtual_key(), 0x52);
        assert_eq!(Key::Digit('0').virtual_key(), 0x30);
        assert_eq!(Key::F(1).virtual_key(), 0x70);
        assert_eq!(Key::F(24).virtual_key(), 0x87);
    }

    #[test]
    fn messages_are_the_spec_strings() {
        assert_eq!(
            HotkeyError::InUse {
                hotkey: "Ctrl+Alt+R".into()
            }
            .to_string(),
            "Ctrl+Alt+R is already used by another app. Choose another shortcut."
        );
        let altgr = HotkeyError::AltGr {
            hotkey: "Ctrl+Alt+C".into(),
            character: 'ć',
        };
        assert_eq!(
            altgr.to_string(),
            "That shortcut types a character on your keyboard layout. Choose another."
        );
        assert_eq!(
            altgr.to_json(),
            serde_json::json!({"kind": "HotkeyInvalid",
                "message": "That shortcut types a character on your keyboard layout. Choose another.",
                "hotkey": "Ctrl+Alt+C", "character": "ć"})
        );
        assert_eq!(
            normalise(" Shift+R ").unwrap_err().to_json(),
            serde_json::json!({"kind": "HotkeyInvalid",
                "message": "Use Ctrl, Alt or the Windows key, with Shift if you like, and one letter, digit or F1 to F24.",
                "hotkey": "Shift+R"})
        );
    }

    #[test]
    fn a_refused_change_keeps_the_old_hotkey() {
        let fake = Fake {
            refuse: vec!["Ctrl+Alt+K".into()],
            ..Fake::default()
        };
        let mut h = Hotkeys::new(DEFAULT_CHECK);
        h.start(&fake, false).unwrap();
        assert_eq!(fake.registered(), ["Ctrl+Alt+R"]);
        let e = h.change("Ctrl+Alt+K", &fake, &Polish).unwrap_err();
        assert_eq!(
            e,
            HotkeyError::InUse {
                hotkey: "Ctrl+Alt+K".into()
            }
        );
        let e = h.change("Ctrl+Alt+C", &fake, &Polish).unwrap_err();
        assert!(matches!(
            e,
            HotkeyError::AltGr {
                character: 'ć', ..
            }
        ));
        assert_eq!(fake.registered(), ["Ctrl+Alt+R"]);
        assert_eq!(h.check(), "Ctrl+Alt+R");
        h.change("Ctrl+Shift+Y", &fake, &Polish).unwrap();
        assert_eq!(fake.registered(), ["Ctrl+Shift+Y"]);
        assert_eq!(h.check(), "Ctrl+Shift+Y");
    }

    #[test]
    fn pause_unregisters_everything_and_resume_brings_it_back() {
        let fake = Fake::default();
        let mut h = Hotkeys::new(DEFAULT_CHECK);
        h.start(&fake, false).unwrap();
        h.pause(&fake);
        assert!(h.is_paused());
        assert!(fake.registered().is_empty());
        // A change while paused is remembered and registered on resume.
        h.change("Ctrl+Alt+J", &fake, &NoAltGr).unwrap();
        assert!(fake.registered().is_empty());
        h.resume(&fake).unwrap();
        assert_eq!(fake.registered(), ["Ctrl+Alt+J"]);
        // Paused at start: nothing registered.
        let fake = Fake::default();
        let mut h = Hotkeys::new(DEFAULT_CHECK);
        h.start(&fake, true).unwrap();
        assert!(fake.registered().is_empty());
    }

    #[test]
    fn the_same_shortcut_is_registered_again_after_a_refusal() {
        let fake = Fake {
            refuse: vec!["Ctrl+Alt+R".into()],
            ..Fake::default()
        };
        let mut h = Hotkeys::new(DEFAULT_CHECK);
        assert!(h.start(&fake, false).is_err());
        // Still held by the other app: refused again, naming it.
        assert_eq!(
            h.change("ctrl+alt+r", &fake, &NoAltGr),
            Err(HotkeyError::InUse {
                hotkey: "Ctrl+Alt+R".into()
            })
        );
        // The other app let go of it.
        let fake = Fake::default();
        h.change("Ctrl+Alt+R", &fake, &NoAltGr).unwrap();
        assert_eq!(fake.registered(), ["Ctrl+Alt+R"]);
        // Registered already: nothing more is asked of the system.
        h.change("Ctrl+Alt+R", &fake, &NoAltGr).unwrap();
        assert_eq!(fake.registered(), ["Ctrl+Alt+R"]);
    }

    #[test]
    fn a_start_refused_by_the_system_names_the_hotkey() {
        let fake = Fake {
            refuse: vec!["Ctrl+Alt+R".into()],
            ..Fake::default()
        };
        let mut h = Hotkeys::new(DEFAULT_CHECK);
        assert_eq!(
            h.start(&fake, false),
            Err(HotkeyError::InUse {
                hotkey: "Ctrl+Alt+R".into()
            })
        );
    }
}
