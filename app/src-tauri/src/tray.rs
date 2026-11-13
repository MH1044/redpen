//! The tray icon and its menu. From module 03 the menu starts with a disabled item showing the
//! publik balance line (docs/PUBLIK-INTEGRATION.md §9, "$3.12 of publik balance") once publik
//! API is set up on this computer; before that there is no such item. Settings and Pause arrive
//! with module 06.
//!
//! What the menu holds is decided by [`menu_items`], without Tauri, so tests check it; [`Tray`]
//! builds it and keeps the balance item up to date as the status changes.

use std::sync::Mutex;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

use crate::publik::balance_line;
use crate::status::PublikStatus;

/// The menu item that quits the app.
pub const QUIT_ID: &str = "quit";
/// The disabled item with the balance line.
pub const BALANCE_ID: &str = "balance";

/// One menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayItem {
    pub id: &'static str,
    pub label: String,
    pub enabled: bool,
}

/// The balance line the tray shows for this status; `None` while publik API is not set up.
pub fn balance_label(status: Option<&PublikStatus>) -> Option<String> {
    status.map(|s| balance_line(s.balance_micros, s.claim_state))
}

/// The tray menu: the balance line (disabled) when there is one, then Quit.
pub fn menu_items(balance: Option<&str>) -> Vec<TrayItem> {
    let mut items = Vec::new();
    if let Some(label) = balance {
        items.push(TrayItem {
            id: BALANCE_ID,
            label: label.to_string(),
            enabled: false,
        });
    }
    items.push(TrayItem {
        id: QUIT_ID,
        label: "Quit".into(),
        enabled: true,
    });
    items
}

/// The tray's menu, kept so the balance item can change.
pub struct Tray<R: Runtime> {
    menu: Menu<R>,
    balance: Mutex<Option<MenuItem<R>>>,
}

impl<R: Runtime> Tray<R> {
    /// Builds the tray icon with [`menu_items`] for `balance`.
    pub fn build(app: &AppHandle<R>, balance: Option<String>) -> tauri::Result<Self> {
        let mut built = Vec::new();
        let mut balance_item = None;
        for item in menu_items(balance.as_deref()) {
            let menu_item =
                MenuItem::with_id(app, item.id, &item.label, item.enabled, None::<&str>)?;
            if item.id == BALANCE_ID {
                balance_item = Some(menu_item.clone());
            }
            built.push(menu_item);
        }
        let refs: Vec<&dyn tauri::menu::IsMenuItem<R>> = built
            .iter()
            .map(|i| i as &dyn tauri::menu::IsMenuItem<R>)
            .collect();
        let menu = Menu::with_items(app, &refs)?;
        let icon = app
            .default_window_icon()
            .cloned()
            .expect("the app icon is embedded at build time");
        TrayIconBuilder::with_id("redpen")
            .icon(icon)
            .tooltip("Redpen")
            .menu(&menu)
            .show_menu_on_left_click(true)
            .on_menu_event(|app, event| {
                if event.id().as_ref() == QUIT_ID {
                    app.exit(0);
                }
            })
            .build(app)?;
        Ok(Self {
            menu,
            balance: Mutex::new(balance_item),
        })
    }

    /// Shows `label` as the balance item, adding it at the top if it is not there, or removes
    /// the item when `label` is `None`.
    pub fn show_balance(&self, app: &AppHandle<R>, label: Option<String>) {
        let mut current = self.balance.lock().unwrap_or_else(|e| e.into_inner());
        let result = match (label, current.as_ref()) {
            (Some(label), Some(item)) => item.set_text(label),
            (Some(label), None) => {
                MenuItem::with_id(app.app_handle(), BALANCE_ID, label, false, None::<&str>)
                    .and_then(|item| {
                        self.menu.prepend(&item)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::provider::ClaimState;

    #[test]
    fn no_balance_item_before_publik_is_set_up() {
        assert_eq!(balance_label(None), None);
        assert_eq!(
            menu_items(None),
            vec![TrayItem {
                id: "quit",
                label: "Quit".into(),
                enabled: true
            }]
        );
    }

    #[test]
    fn the_balance_line_is_a_disabled_first_item() {
        let status = PublikStatus::new(ClaimState::Claimed, 3_120_000, 0);
        let label = balance_label(Some(&status)).unwrap();
        assert_eq!(label, "$3.12 of publik balance");
        let items = menu_items(Some(&label));
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            TrayItem {
                id: "balance",
                label: "$3.12 of publik balance".into(),
                enabled: false
            }
        );
        assert_eq!(items[1].id, "quit");
        let fresh = PublikStatus::new(ClaimState::Anonymous, 0, 0);
        assert_eq!(
            balance_label(Some(&fresh)).unwrap(),
            "$0.00 · link this computer for $0.05 of free use"
        );
    }
}
