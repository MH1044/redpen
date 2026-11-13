//! The Tauri commands, one file per command group of docs/ARCHITECTURE.md §5. Each is a thin
//! wrapper over a Tauri-free function, and every result is a keyless type (rule A1): the web
//! view never receives a key.

pub mod publik;
pub mod session;
pub mod settings;

use redpen_core::provider::AiError;
use serde::Serialize;
use serde_json::{Value, json};

use crate::publik::LinkError;

/// An error as the interface receives it: `{"kind", "message"}` plus the error's fields.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CommandError(pub Value);

impl CommandError {
    /// `{"kind": "Invalid", "message": …}`.
    pub fn invalid(message: impl Into<String>) -> Self {
        CommandError(json!({"kind": "Invalid", "message": message.into()}))
    }

    /// The error's kind.
    pub fn kind(&self) -> Option<&str> {
        self.0.get("kind").and_then(Value::as_str)
    }
}

impl From<AiError> for CommandError {
    fn from(e: AiError) -> Self {
        CommandError(e.to_json())
    }
}

impl From<LinkError> for CommandError {
    fn from(e: LinkError) -> Self {
        CommandError(serde_json::to_value(e).expect("a link error serialises"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_reach_the_interface_as_kind_and_message() {
        let e = CommandError::from(AiError::NoAppToken);
        assert_eq!(e.kind(), Some("NoAppToken"));
        assert_eq!(
            e.0["message"],
            "This build has no publik API app token. Use your own key for now."
        );
        let e = CommandError::from(LinkError::NoLink);
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"kind": "NoLink", "message": "There is no publik link to open right now."})
        );
        assert_eq!(
            CommandError::invalid("Settings were written by a newer Redpen.").kind(),
            Some("Invalid")
        );
    }
}
