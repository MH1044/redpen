//! The tray icon and its menu (docs/PRODUCT.md §5): **Open Settings**; the publik balance line
//! (docs/PUBLIK-INTEGRATION.md §9, "$3.12 of publik balance"), a disabled item shown only once
//! publik API is set up on this computer; **Pause Redpen**, a check item that lets go of every
//! hotkey and greys the icon; **Quit**.
//!
//! What the menu holds is decided by [`menu_items`], and the grey icon by [`greyed`], without
//! Tauri, so tests check them; [`Tray`] builds the menu and keeps the balance item, the pause
//! item and the icon up to date.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Runtime};

use crate::publik::balance_line;
use crate::status::PublikStatus;

/// The item that opens Settings.
pub const SETTINGS_ID: &str = "settings";
/// The disabled item with the balance line.
pub const BALANCE_ID: &str = "balance";
/// The check item that pauses Redpen.
pub const PAUSE_ID: &str = "pause";
/// The menu item that quits the app.
pub const QUIT_ID: &str = "quit";

/// One menu item. `checked` is `Some` for a check item (Pause Redpen, whose tick [`Tray`] sets
/// from the settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayItem {
    pub id: &'static str,
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
}

impl TrayItem {
    fn plain(id: &'static str, label: &str) -> Self {
        Self {
            id,
            label: label.into(),
            enabled: true,
            checked: None,
        }
    }
}

/// The balance line the tray shows for this status; `None` while publik API is not set up.
pub fn balance_label(status: Option<&PublikStatus>) -> Option<String> {
    status.map(|s| balance_line(s.balance_micros, s.claim_state))
}

/// The tray menu: Open Settings, the balance line (disabled) when there is one, Pause Redpen (a
/// check item), Quit.
pub fn menu_items(balance: Option<&str>) -> Vec<TrayItem> {
    let mut items = vec![TrayItem::plain(SETTINGS_ID, "Open Settings")];
    if let Some(label) = balance {
        items.push(TrayItem {
            id: BALANCE_ID,
            label: label.to_string(),
            enabled: false,
            checked: None,
        });
    }
    items.push(TrayItem {
        checked: Some(false),
        ..TrayItem::plain(PAUSE_ID, "Pause Redpen")
    });
    items.push(TrayItem::plain(QUIT_ID, "Quit"));
    items
}

/// The icon's pixels in grey, for Pause Redpen: each pixel's luminance (ITU-R BT.601 weights,
/// in whole numbers) lifted halfway towards white, so the paused icon reads as switched off;
/// alpha is kept. `rgba` is four bytes a pixel; a trailing partial pixel is left as it is.
pub fn greyed(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.as_chunks_mut::<4>().0 {
        let luma =
            (299 * u32::from(px[0]) + 587 * u32::from(px[1]) + 114 * u32::from(px[2])) / 1000;
        let light = u8::try_from((luma + 255) / 2).unwrap_or(u8::MAX);
        px[0] = light;
        px[1] = light;
        px[2] = light;
    }
    out
}

/// What the tray menu does when an item is chosen.
pub type MenuHandler<R> = fn(&AppHandle<R>, &str);

/// The tray icon, with what it needs to change later.
pub struct Tray<R: Runtime> {
    menu: Menu<R>,
    icon: TrayIcon<R>,
    normal: Image<'static>,
    balance: Mutex<Option<MenuItem<R>>>,
    pause: CheckMenuItem<R>,
}

impl<R: Runtime> Tray<R> {
    /// Builds the tray icon with [`menu_items`] for `balance`, Pause Redpen ticked and the icon
    /// grey when `paused`; `on_item` runs with the id of the item chosen.
    pub fn build(
        app: &AppHandle<R>,
        balance: Option<String>,
        paused: bool,
        on_item: MenuHandler<R>,
    ) -> tauri::Result<Self> {
        let menu = Menu::new(app)?;
        let mut balance_item = None;
        let mut pause_item = None;
        for item in menu_items(balance.as_deref()) {
            if item.checked.is_some() {
                let check = CheckMenuItem::with_id(
                    app,
                    item.id,
                    &item.label,
                    item.enabled,
                    paused,
                    None::<&str>,
                )?;
                menu.append(&check)?;
                pause_item = Some(check);
                continue;
            }
            let plain = MenuItem::with_id(app, item.id, &item.label, item.enabled, None::<&str>)?;
            menu.append(&plain)?;
            if item.id == BALANCE_ID {
                balance_item = Some(plain);
            }
        }
        let normal = app
            .default_window_icon()
            .cloned()
            .expect("the app icon is embedded at build time")
            .to_owned();
        let icon = TrayIconBuilder::with_id("redpen")
            .icon(if paused {
                grey_image(&normal)
            } else {
                normal.clone()
            })
            .tooltip(tooltip(paused))
            .menu(&menu)
            .show_menu_on_left_click(true)
            .on_menu_event(move |app, event| on_item(app, event.id().as_ref()))
            .build(app)?;
        Ok(Self {
            menu,
            icon,
            normal,
            balance: Mutex::new(balance_item),
            pause: pause_item.expect("the menu has a pause item"),
        })
    }

    /// Shows `label` as the balance item, adding it after Open Settings if it is not there, or
    /// removes the item when `label` is `None`.
    pub fn show_balance(&self, app: &AppHandle<R>, label: Option<String>) {
        let mut current = self.balance.lock().unwrap_or_else(|e| e.into_inner());
        let result = match (label, current.as_ref()) {
            (Some(label), Some(item)) => item.set_text(label),
            (Some(label), None) => {
                MenuItem::with_id(app.app_handle(), BALANCE_ID, label, false, None::<&str>)
                    .and_then(|item| {
                        self.menu.insert(&item as &dyn IsMenuItem<R>, 1)?;
                        *current = Some(item);
                        Ok(())
                    })
            }
            (None, Some(item)) => {
                let removed = self.menu.remove(item);
                *current = None;
                removed
            }
            (None, None) => Ok(()),
        };
        if result.is_err() {
            tracing::warn!("the tray's balance item could not be updated");
        }
    }

    /// Ticks Pause Redpen and greys the icon while paused; the normal icon otherwise.
    pub fn show_paused(&self, paused: bool) {
        let icon = if paused {
            grey_image(&self.normal)
        } else {
            self.normal.clone()
        };
        let result = self
            .pause
            .set_checked(paused)
            .and_then(|()| self.icon.set_icon(Some(icon)))
            .and_then(|()| self.icon.set_tooltip(Some(tooltip(paused))));
        if result.is_err() {
            tracing::warn!("the tray could not show the pause state");
        }
    }
}

fn tooltip(paused: bool) -> &'static str {
    if paused { "Redpen (paused)" } else { "Redpen" }
}

fn grey_image(normal: &Image<'_>) -> Image<'static> {
    Image::new_owned(greyed(normal.rgba()), normal.width(), normal.height())
}

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::provider::ClaimState;

    fn ids(items: &[TrayItem]) -> Vec<&'static str> {
        items.iter().map(|i| i.id).collect()
    }

    #[test]
    fn no_balance_item_before_publik_is_set_up() {
        assert_eq!(balance_label(None), None);
        let items = menu_items(None);
        assert_eq!(ids(&items), ["settings", "pause", "quit"]);
        assert_eq!(items[0].label, "Open Settings");
        assert_eq!(
            items[1],
            TrayItem {
                id: "pause",
                label: "Pause Redpen".into(),
                enabled: true,
                checked: Some(false)
            }
        );
        assert_eq!(items[2].label, "Quit");
    }

    #[test]
    fn the_balance_line_is_a_disabled_item_after_open_settings() {
        let status = PublikStatus::new(ClaimState::Claimed, 3_120_000, 0);
        let label = balance_label(Some(&status)).unwrap();
        assert_eq!(label, "$3.12 of publik balance");
        let items = menu_items(Some(&label));
        assert_eq!(ids(&items), ["settings", "balance", "pause", "quit"]);
        assert_eq!(
            items[1],
            TrayItem {
                id: "balance",
                label: "$3.12 of publik balance".into(),
                enabled: false,
                checked: None
            }
        );
        let fresh = PublikStatus::new(ClaimState::Anonymous, 0, 0);
        assert_eq!(
            balance_label(Some(&fresh)).unwrap(),
            "$0.00 · link this computer for $0.05 of free use"
        );
    }

    #[test]
    fn pause_is_the_one_check_item() {
        let items = menu_items(Some("$0.00 of publik balance"));
        let checks: Vec<&str> = items
            .iter()
            .filter(|i| i.checked.is_some())
            .map(|i| i.id)
            .collect();
        assert_eq!(checks, ["pause"]);
    }

    #[test]
    fn the_paused_icon_is_grey_and_keeps_its_alpha() {
        // A red, a black and a transparent white pixel.
        let rgba = [255, 0, 0, 255, 0, 0, 0, 200, 255, 255, 255, 0];
        let grey = greyed(&rgba);
        // Red: luminance 76, halfway to white 165.
        assert_eq!(&grey[0..4], &[165, 165, 165, 255]);
        assert_eq!(&grey[4..8], &[127, 127, 127, 200]);
        assert_eq!(&grey[8..12], &[255, 255, 255, 0]);
        // Every pixel is grey.
        assert!(grey.chunks(4).all(|p| p[0] == p[1] && p[1] == p[2]));
    }
}
