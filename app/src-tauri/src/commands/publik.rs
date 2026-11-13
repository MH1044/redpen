//! The publik API commands (docs/ARCHITECTURE.md §5): `consent_info`, `publik_connect`,
//! `publik_disconnect`, `publik_refresh`, `provider_status`, `open_account_link` and
//! `set_providers`. Each wraps a [`PublikService`] method; none returns a key.

use redpen_core::store::Settings;
use redpen_core::store::settings::{ChatChoice, RewriteQuality, SearchChoice};
use tauri::State;

use super::CommandError;
use crate::AppState;
use crate::publik::{
    self, ConnectOutcome, ConsentInfo, LinkError, LinkKind, ProviderStatus, ProvidersPatch,
    PublikService, PublikStatus,
};

/// The consent view's title, data-path sentence and disclosure.
#[tauri::command]
pub fn consent_info() -> ConsentInfo {
    publik::consent_info()
}

/// **Continue with publik API**: sets up this computer and returns the first-run card.
#[tauri::command]
pub async fn publik_connect(state: State<'_, AppState>) -> Result<ConnectOutcome, CommandError> {
    Ok(state.publik.connect().await?)
}

/// **Disconnect**: revokes the key (best effort) and forgets it.
#[tauri::command]
pub async fn publik_disconnect(state: State<'_, AppState>) -> Result<(), CommandError> {
    state.publik.disconnect().await;
    Ok(())
}

/// Settings opening: `GET /wallet`, only while publik API is chosen and set up.
#[tauri::command]
pub async fn publik_refresh(state: State<'_, AppState>) -> Result<PublikStatus, CommandError> {
    Ok(state.publik.refresh().await?)
}

/// The chosen AI provider and the publik status.
#[tauri::command]
pub fn provider_status(state: State<'_, AppState>) -> ProviderStatus {
    state.publik.provider_status()
}

/// Opens the claim, add-credit or top-up link in the system browser. The interface names the
/// kind of link; it can never pass a URL.
#[tauri::command]
pub fn open_account_link(state: State<'_, AppState>, kind: LinkKind) -> Result<(), CommandError> {
    Ok(open_link(&state.publik, kind)?)
}

/// Settings → Providers: the AI provider, Rewrite quality and Web search.
#[tauri::command(rename_all = "snake_case")]
pub fn set_providers(
    state: State<'_, AppState>,
    chat: Option<ChatChoice>,
    rewrite_quality: Option<RewriteQuality>,
    search: Option<SearchChoice>,
) -> Result<Settings, CommandError> {
    let patch = ProvidersPatch {
        chat,
        rewrite_quality,
        search,
    };
    state
        .publik
        .set_providers(patch)
        .map_err(|e| CommandError::invalid(e.to_string()))
}

/// The link of `kind`, handed to the system browser.
pub fn open_link(service: &PublikService, kind: LinkKind) -> Result<(), LinkError> {
    let url = service.account_link(kind)?;
    browser::open(&url).map_err(|_| {
        tracing::warn!("the system browser could not be started");
        LinkError::CouldNotOpen
    })
}

/// The system browser. The URL is an `https` address without spaces or control characters
/// (`PublikService::account_link` checks it), handed over as one argument, never through a shell.
mod browser {
    #[cfg(windows)]
    pub fn open(url: &str) -> std::io::Result<()> {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::{HSTRING, PCWSTR, w};
        let file = HSTRING::from(url);
        // SAFETY: every pointer is a valid null-terminated wide string for the call's duration.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &file,
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW reports success with a value above 32.
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(std::io::Error::other("ShellExecuteW failed"))
        }
    }

    #[cfg(not(windows))]
    pub fn open(url: &str) -> std::io::Result<()> {
        let mut child = std::process::Command::new("xdg-open")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publik::{PublikOptions, SharedSettings};
    use crate::status::StatusStore;
    use redpen_core::clock::FakeClock;
    use redpen_core::net::FakeTransport;
    use std::sync::{Arc, RwLock};

    #[test]
    fn no_pending_402_means_no_link_and_nothing_opens() {
        let dir = tempfile::tempdir().unwrap();
        let settings: SharedSettings = Arc::new(RwLock::new(Settings::default()));
        let fake = Arc::new(FakeTransport::scripted(Vec::new()));
        let service = PublikService::new(
            fake.clone(),
            PublikOptions {
                credential_path: dir.path().join("redpen.json"),
                app_token: None,
                base_url: "https://publikhq.com/api/v1".into(),
                app_version: "0.1.0".into(),
                new_install_id: || "id".into(),
                env: |_| None,
            },
            dir.path().join("config"),
            settings,
            Arc::new(StatusStore::new(dir.path(), Arc::new(FakeClock::at(0)))),
        );
        for kind in [LinkKind::TopUp, LinkKind::Claim, LinkKind::AddCredit] {
            assert_eq!(open_link(&service, kind), Err(LinkError::NoLink));
        }
        assert!(fake.requests_seen().is_empty());
    }

    #[test]
    fn a_link_kind_is_one_of_three_names_never_a_url() {
        for (name, kind) in [
            ("claim", LinkKind::Claim),
            ("add_credit", LinkKind::AddCredit),
            ("top_up", LinkKind::TopUp),
        ] {
            assert_eq!(
                serde_json::from_value::<LinkKind>(serde_json::json!(name)).unwrap(),
                kind
            );
        }
        assert!(
            serde_json::from_value::<LinkKind>(serde_json::json!("https://evil.example/")).is_err()
        );
        assert!(serde_json::from_value::<ChatChoice>(serde_json::json!("publik")).is_ok());
    }
}
