//! Session commands (docs/ARCHITECTURE.md §5): `session_get`, `apply_fix`, `rewrite`, `accept`,
//! `copy_result`, `undo_last_replace` and `close_session`, each a thin wrapper over
//! [`redpen_core::flow::Sessions`], and what the check shortcut does ([`on_check_shortcut`]):
//! capture, Harper, then `popup://open`.
//!
//! Two errors carry what the popup needs to offer its one way out, so the web view never asks
//! for it separately: `NeedsConsent` carries the consent view's strings (`consent`), and
//! `NeedsCredit` the claim state (`claim_state`), which names the link ("Sign in with publik"
//! while anonymous, "Add a plan or pack" once claimed). Neither carries a key.

use std::sync::atomic::Ordering;

use redpen_core::flow::{RewriteResult, Session, SessionError, Sessions, UndoOutcome};
use redpen_core::provider::{ClaimState, ProviderKind, ProviderResolver};
use redpen_core::rewrite::Action;
use redpen_core::store::Settings;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, Runtime, State};

use super::CommandError;
use crate::AppState;
use crate::publik::{balance_line, consent_info};
use crate::windows;

/// What `rewrite` returns: the result, and on publik API the balance line after the call (from
/// the status its headers updated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RewriteView {
    #[serde(flatten)]
    pub result: RewriteResult,
    pub balance_line: Option<String>,
}

/// The message of a press of the shortcut on a system where Redpen cannot read a selection yet
/// (Linux, until module 07).
pub const CAPTURE_UNAVAILABLE: &str =
    "Redpen can't read a selection on this system yet. Checking selections works on Windows.";

/// A session error as the popup receives it: `{kind, message, ...}`, plus the consent view's
/// strings for `NeedsConsent` and the claim state for `NeedsCredit`.
pub fn session_error(e: &SessionError, claim_state: ClaimState) -> CommandError {
    let mut value = e.to_json();
    match e.kind() {
        "NeedsConsent" => {
            value["consent"] = serde_json::to_value(consent_info()).expect("consent serialises");
        }
        "NeedsCredit" => {
            value["claim_state"] = json!(claim_state);
        }
        _ => {}
    }
    CommandError(value)
}

fn error(state: &AppState, e: &SessionError) -> CommandError {
    session_error(e, state.resolver.tier_context().claim_state)
}

#[tauri::command(rename_all = "snake_case")]
pub fn session_get(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Session, CommandError> {
    state
        .sessions
        .get(&session_id)
        .map_err(|e| error(&state, &e))
}

#[tauri::command(rename_all = "snake_case")]
pub fn apply_fix(
    state: State<'_, AppState>,
    session_id: String,
    finding_id: String,
    suggestion_index: usize,
) -> Result<Session, CommandError> {
    let settings = state.settings.get();
    state
        .sessions
        .apply_fix(&session_id, &finding_id, suggestion_index, &settings)
        .map_err(|e| error(&state, &e))
}

/// One of the six actions on the session's text. `variant` counts presses of Another version
/// (0 when absent); the style is the author's own until module 08.
#[tauri::command(rename_all = "snake_case")]
pub async fn rewrite(
    state: State<'_, AppState>,
    session_id: String,
    action: Action,
    instruction: Option<String>,
    variant: Option<u32>,
) -> Result<RewriteView, CommandError> {
    let settings = state.settings.get();
    let result = state
        .sessions
        .rewrite(
            &session_id,
            action,
            instruction,
            variant.unwrap_or(0),
            &settings,
        )
        .await
        .map_err(|e| error(&state, &e))?;
    let balance_line = (result.provider == ProviderKind::Publik)
        .then(|| state.publik.status().current())
        .flatten()
        .map(|s| balance_line(s.balance_micros, s.claim_state));
    Ok(RewriteView {
        result,
        balance_line,
    })
}

/// Accept: the popup steps aside, the text is typed into the window the selection came from,
/// and the session closes. When Accept stops, the popup comes back with the message.
#[tauri::command(rename_all = "snake_case")]
pub async fn accept(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    result_id: Option<String>,
) -> Result<(), CommandError> {
    let settings = state.settings.get();
    windows::hide_popup(&app);
    match state
        .sessions
        .accept(&session_id, result_id.as_deref(), &settings)
        .await
    {
        Ok(()) => {
            state.sessions.close(&session_id);
            Ok(())
        }
        Err(e) => {
            windows::reshow_popup(&app);
            Err(error(&state, &e))
        }
    }
}

/// Copy: the result (or, without `result_id`, the session's text) on the clipboard.
#[tauri::command(rename_all = "snake_case")]
pub async fn copy_result(
    state: State<'_, AppState>,
    session_id: String,
    result_id: Option<String>,
) -> Result<(), CommandError> {
    state
        .sessions
        .copy_result(&session_id, result_id.as_deref())
        .await
        .map_err(|e| error(&state, &e))
}

/// Undo: the popup steps aside and the last Accept is taken back; the popup's session, if any,
/// closes with it. When the window could not take the text back and the original went on the
/// clipboard instead, the popup comes back to say so.
#[tauri::command]
pub async fn undo_last_replace(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UndoOutcome, CommandError> {
    windows::hide_popup(&app);
    match state.sessions.undo().await {
        Ok(outcome) => {
            state.sessions.close_all();
            if outcome == UndoOutcome::OnClipboard {
                windows::reshow_popup(&app);
            }
            Ok(outcome)
        }
        Err(e) => {
            windows::reshow_popup(&app);
            Err(error(&state, &e))
        }
    }
}

/// Esc, or the popup closing: the text is dropped from memory and the popup hides.
#[tauri::command(rename_all = "snake_case")]
pub fn close_session(app: AppHandle, state: State<'_, AppState>, session_id: String) {
    state.sessions.close(&session_id);
    windows::hide_popup(&app);
}

/// The `popup://open` payload: the session to show, or the reason the capture stopped, and
/// whether the last Accept can still be undone.
pub fn popup_open_payload(result: &Result<Session, Value>, can_undo: bool) -> Value {
    match result {
        Ok(s) => json!({"session_id": s.id, "can_undo": can_undo}),
        Err(e) => json!({"session_id": null, "error": e, "can_undo": can_undo}),
    }
}

/// The capture and the check, from a press of the shortcut. One popup at a time: the session
/// it showed before is closed first.
pub async fn capture_and_check(state: &AppState) -> Result<Session, Value> {
    open_on_shortcut(&state.sessions, &state.settings.get(), cfg!(windows)).await
}

/// What a press of the shortcut opens, without Tauri: every open session is closed, then the
/// selection is captured and checked. Where Redpen cannot read a selection yet (`can_capture`
/// false: Linux, until module 07) nothing is read and the answer is `CaptureUnavailable`.
pub async fn open_on_shortcut(
    sessions: &Sessions,
    settings: &Settings,
    can_capture: bool,
) -> Result<Session, Value> {
    sessions.close_all();
    if !can_capture {
        return Err(json!({"kind": "CaptureUnavailable", "message": CAPTURE_UNAVAILABLE}));
    }
    sessions.open(settings).await.map_err(|e| e.to_json())
}

/// What a press of the check shortcut does: nothing while the popup has the focus (its own keys
/// are for it) or while the last press is still capturing; otherwise capture and check, then the
/// popup shows the session (below the caret, else at the mouse) or the reason it stopped.
pub fn on_check_shortcut<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if windows::popup_in_front(app) || state.capturing.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let result = capture_and_check(&state).await;
        state.capturing.store(false, Ordering::SeqCst);
        let anchor = result
            .as_ref()
            .ok()
            .and_then(|s| state.sessions.anchor(&s.id));
        windows::emit_popup(&app, popup_open_payload(&result, state.sessions.can_undo()));
        match anchor {
            Some((caret, cursor)) => windows::show_popup(&app, caret, Some(cursor)),
            None => windows::show_popup(&app, None, None),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::platform::CaptureError;
    use redpen_core::provider::AiError;

    #[test]
    fn needs_consent_carries_the_consent_strings() {
        let e = session_error(
            &SessionError::from(AiError::NeedsConsent),
            ClaimState::Anonymous,
        );
        assert_eq!(e.kind(), Some("NeedsConsent"));
        assert_eq!(e.0["consent"]["title"], "Choose how Redpen uses AI");
        assert_eq!(e.0["consent"]["data_path"], crate::publik::DATA_PATH);
        assert_eq!(e.0["consent"]["disclosure"], crate::publik::DISCLOSURE);
    }

    #[test]
    fn needs_credit_carries_the_claim_state_and_one_link() {
        let e = session_error(
            &SessionError::from(AiError::NeedsCredit {
                message: "Your publik balance is $0.00.".into(),
                top_up_url: Some("https://publikhq.com/claim/abc".into()),
            }),
            ClaimState::Claimed,
        );
        assert_eq!(e.0["claim_state"], "claimed");
        assert_eq!(e.0["top_up_url"], "https://publikhq.com/claim/abc");
        let other = session_error(&SessionError::from(AiError::Offline), ClaimState::Claimed);
        assert!(other.0.get("claim_state").is_none());
        assert!(other.0.get("consent").is_none());
    }

    #[test]
    fn the_popup_opens_with_a_session_or_the_refusal() {
        let refused: Result<Session, Value> = Err(CaptureError::PasswordField.to_json());
        assert_eq!(
            popup_open_payload(&refused, false),
            json!({"session_id": null, "can_undo": false, "error": {"kind": "PasswordField",
                   "message": "Redpen doesn't read or type into password fields."}})
        );
    }

    #[test]
    fn a_rewrite_view_is_the_result_and_the_balance_line() {
        let view = RewriteView {
            result: RewriteResult {
                result_id: "r1".into(),
                text: "Hi.".into(),
                diff: Vec::new(),
                warnings: Vec::new(),
                accept_default: true,
                unchanged: false,
                from_cache: false,
                provider: ProviderKind::Publik,
                provider_line: "publik-fast".into(),
                charge_micros: Some(210),
            },
            balance_line: Some("$3.12 of publik balance".into()),
        };
        let v = serde_json::to_value(&view).unwrap();
        assert_eq!(v["result_id"], "r1");
        assert_eq!(v["provider_line"], "publik-fast");
        assert_eq!(v["charge_micros"], 210);
        assert_eq!(v["balance_line"], "$3.12 of publik balance");
    }

    #[test]
    fn the_capture_unavailable_message_is_the_spec_sentence() {
        assert_eq!(
            CAPTURE_UNAVAILABLE,
            "Redpen can't read a selection on this system yet. Checking selections works on Windows."
        );
    }
}
