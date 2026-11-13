//! Redpen's windows (docs/PRODUCT.md §5): the popup, Settings and the welcome window. Every
//! window loads the same interface, which shows the screen named by the window's label (`popup`,
//! `settings`, `welcome`).
//!
//! - **Popup**: frameless, always on top, out of the taskbar, made hidden at start so the
//!   shortcut only has to move and show it. It goes where
//!   [`redpen_core::platform::placement::place`] says: below the caret, else at the mouse,
//!   flipped above when there is no room, inside the monitor's work area.
//! - **Settings**: a normal window, made on first use and opened at a tab (and a section of
//!   Providers). A new window learns its tab from [`settings_target_script`]; an open one from
//!   the `settings://open` event.
//! - **Welcome**: on the first launch only ([`shows_welcome`]); `ui.welcomed` is saved as soon as
//!   it opens.

use redpen_core::platform::{Point, Rect, place};
use redpen_core::store::Settings;
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Runtime, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

/// The popup's window label.
pub const POPUP: &str = "popup";
/// Settings' window label.
pub const SETTINGS: &str = "settings";
/// The welcome window's label.
pub const WELCOME: &str = "welcome";

/// The popup's size, in logical pixels.
pub const POPUP_SIZE: (f64, f64) = (460.0, 440.0);

/// The event that tells the popup to show a session (docs/ARCHITECTURE.md §5).
pub const POPUP_OPEN_EVENT: &str = "popup://open";
/// The event that tells an open Settings window which tab to show.
pub const SETTINGS_OPEN_EVENT: &str = "settings://open";

/// Settings' tabs; Dictionary arrives with module 08.
pub const TABS: [&str; 4] = ["general", "providers", "privacy", "about"];
/// Sections of the Providers tab a button can open at.
pub const SECTIONS: [&str; 3] = ["publik", "own_key", "local"];

/// Whether the welcome window opens at this launch: only until it has been shown once.
pub fn shows_welcome(s: &Settings) -> bool {
    !s.ui.welcomed
}

/// Where Settings opens: one of [`TABS`] (General for anything else) and, on Providers, one of
/// [`SECTIONS`] or none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsTarget {
    pub tab: &'static str,
    pub section: Option<&'static str>,
}

impl SettingsTarget {
    /// The target for a tab and section as the interface names them; unknown names are dropped,
    /// so nothing the web view sends ends up in a script.
    pub fn new(tab: &str, section: Option<&str>) -> Self {
        let tab = TABS
            .iter()
            .copied()
            .find(|t| *t == tab)
            .unwrap_or("general");
        let section = section
            .filter(|_| tab == "providers")
            .and_then(|s| SECTIONS.iter().copied().find(|k| *k == s));
        Self { tab, section }
    }
}

/// The script a new Settings window runs before its page: it sets
/// `window.__REDPEN_SETTINGS_TARGET__` to the target's JSON. Only the fixed names of [`TABS`] and
/// [`SECTIONS`] can appear in it.
pub fn settings_target_script(target: &SettingsTarget) -> String {
    let json = serde_json::to_string(target).expect("a target serialises");
    format!("window.__REDPEN_SETTINGS_TARGET__ = {json};")
}

/// Makes the hidden popup window.
pub fn create_popup<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    WebviewWindowBuilder::new(app, POPUP, WebviewUrl::default())
        .title("Redpen")
        .inner_size(POPUP_SIZE.0, POPUP_SIZE.1)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .focused(false)
        .build()
}

/// Every monitor's work area, in physical pixels.
pub fn work_areas<R: Runtime>(window: &WebviewWindow<R>) -> Vec<Rect> {
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let area = m.work_area();
            Rect {
                x: area.position.x,
                y: area.position.y,
                width: i32::try_from(area.size.width).unwrap_or(i32::MAX),
                height: i32::try_from(area.size.height).unwrap_or(i32::MAX),
            }
        })
        .collect()
}

/// The popup's size in physical pixels at `scale`.
pub fn popup_physical_size(scale: f64) -> (u32, u32) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (
        (POPUP_SIZE.0 * scale).round() as u32,
        (POPUP_SIZE.1 * scale).round() as u32,
    )
}

/// Moves the popup next to `caret` (or `cursor`), shows it and gives it the focus. Without a
/// cursor (a capture that stopped), the mouse as the system reports it now.
pub fn show_popup<R: Runtime>(app: &AppHandle<R>, caret: Option<Rect>, cursor: Option<Point>) {
    let Some(window) = app.get_webview_window(POPUP) else {
        tracing::warn!("the popup window is missing");
        return;
    };
    let cursor = cursor.unwrap_or_else(|| {
        app.cursor_position()
            .map(|p| Point {
                x: p.x.round() as i32,
                y: p.y.round() as i32,
            })
            .unwrap_or_default()
    });
    // The scale of the monitor the popup goes to: the one under the caret or the mouse.
    let anchor = caret.map(|c| (c.x, c.y)).unwrap_or((cursor.x, cursor.y));
    let scale = window
        .monitor_from_point(f64::from(anchor.0), f64::from(anchor.1))
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .or_else(|| window.scale_factor().ok())
        .unwrap_or(1.0);
    let at = place(
        caret,
        cursor,
        &work_areas(&window),
        popup_physical_size(scale),
    );
    let shown = window
        .set_size(LogicalSize::new(POPUP_SIZE.0, POPUP_SIZE.1))
        .and_then(|()| window.set_position(PhysicalPosition::new(at.x, at.y)))
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus());
    if shown.is_err() {
        tracing::warn!("the popup could not be shown");
    }
}

/// Hides the popup (Esc, Accept, Undo).
pub fn hide_popup<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(POPUP)
        && w.hide().is_err()
    {
        tracing::warn!("the popup could not be hidden");
    }
}

/// Shows the popup again where it was (Accept or Undo stopped).
pub fn reshow_popup<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(POPUP)
        && w.show().and_then(|()| w.set_focus()).is_err()
    {
        tracing::warn!("the popup could not be shown again");
    }
}

/// Whether the popup is on the screen with the focus.
pub fn popup_in_front<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window(POPUP)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

/// Tells the popup what to show (`popup://open`).
pub fn emit_popup<R: Runtime>(app: &AppHandle<R>, payload: serde_json::Value) {
    if app.emit_to(POPUP, POPUP_OPEN_EVENT, payload).is_err() {
        tracing::warn!("the popup event could not be sent");
    }
}

/// Opens Settings at `target`: brings the window forward when it is open, otherwise makes it.
pub fn open_settings<R: Runtime>(app: &AppHandle<R>, target: &SettingsTarget) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.unminimize();
        w.show()?;
        w.set_focus()?;
        app.emit_to(SETTINGS, SETTINGS_OPEN_EVENT, target)?;
        return Ok(());
    }
    WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::default())
        .title("Redpen Settings")
        .inner_size(720.0, 620.0)
        .min_inner_size(560.0, 440.0)
        .initialization_script(settings_target_script(target))
        .focused(true)
        .build()?;
    Ok(())
}

/// Opens the welcome window.
pub fn open_welcome<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(WELCOME) {
        w.show()?;
        return w.set_focus();
    }
    WebviewWindowBuilder::new(app, WELCOME, WebviewUrl::default())
        .title("Welcome to Redpen")
        .inner_size(500.0, 360.0)
        .resizable(false)
        .center()
        .focused(true)
        .build()?;
    Ok(())
}

/// Closes a window by label, if it is open.
pub fn close_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(w) = app.get_webview_window(label)
        && w.close().is_err()
    {
        tracing::warn!("a window could not be closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_welcome_window_opens_until_it_has_been_shown() {
        let mut s = Settings::default();
        assert!(shows_welcome(&s));
        s.ui.welcomed = true;
        assert!(!shows_welcome(&s));
    }

    #[test]
    fn settings_open_only_at_known_tabs_and_sections() {
        assert_eq!(
            SettingsTarget::new("providers", Some("own_key")),
            SettingsTarget {
                tab: "providers",
                section: Some("own_key")
            }
        );
        assert_eq!(
            SettingsTarget::new("https://example.com", Some("local")),
            SettingsTarget {
                tab: "general",
                section: None
            }
        );
        assert_eq!(
            SettingsTarget::new("providers", Some("\";alert(1);//")),
            SettingsTarget {
                tab: "providers",
                section: None
            }
        );
        assert_eq!(SettingsTarget::new("privacy", Some("local")).section, None);
    }

    #[test]
    fn the_target_script_holds_only_json() {
        assert_eq!(
            settings_target_script(&SettingsTarget::new("providers", Some("local"))),
            r#"window.__REDPEN_SETTINGS_TARGET__ = {"tab":"providers","section":"local"};"#
        );
        assert_eq!(
            settings_target_script(&SettingsTarget::new("about", None)),
            r#"window.__REDPEN_SETTINGS_TARGET__ = {"tab":"about","section":null};"#
        );
    }

    #[test]
    fn the_popup_size_follows_the_monitor_scale() {
        assert_eq!(popup_physical_size(1.0), (460, 440));
        assert_eq!(popup_physical_size(1.5), (690, 660));
        assert_eq!(popup_physical_size(f64::NAN), (460, 440));
    }
}
