//! `--provider local|own-key|fake` (docs/modules/02-provider-layer.md): the provider a command
//! calls, and the transport it calls it through.
//!
//! - `fake`: the fake provider (`http://fake.invalid/v1`, model `fake-chat`, no key).
//! - `own-key`: the user's own key from `REDPEN_OWN_KEY`, `REDPEN_OWN_BASE_URL` and
//!   `REDPEN_OWN_MODEL`; the key is read into a secret and never printed.
//! - `local`: Ollama at `http://localhost:11434/v1`, with the model Ollama's list gives.
//!
//! With `--recording R`, every provider answers from that `recording.json` through the fake
//! transport, so nothing reaches the network; without it, `fake` answers nothing (every request
//! is unrecorded) and the other two use the network.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use redpen_core::net::{FakeTransport, HttpTransport, ReqwestTransport};
use redpen_core::provider::local::DEFAULT_LOCAL_BASE_URL;
use redpen_core::provider::{ProviderKind, ResolvedProvider, SecretString};

use crate::cmd::Failure;

/// The environment variables of `--provider own-key`.
pub const OWN_KEY_VAR: &str = "REDPEN_OWN_KEY";
pub const OWN_BASE_URL_VAR: &str = "REDPEN_OWN_BASE_URL";
pub const OWN_MODEL_VAR: &str = "REDPEN_OWN_MODEL";

/// A `--provider` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderArg {
    Fake,
    Local,
    OwnKey,
}

impl FromStr for ProviderArg {
    type Err = Failure;

    fn from_str(s: &str) -> Result<Self, Failure> {
        match s {
            "fake" => Ok(ProviderArg::Fake),
            "local" => Ok(ProviderArg::Local),
            "own-key" => Ok(ProviderArg::OwnKey),
            other => Err(Failure::usage(format!(
                "Unknown provider \"{other}\". Use local, own-key or fake."
            ))),
        }
    }
}

/// The provider and the transport a command uses.
pub struct Setup {
    pub provider: ResolvedProvider,
    pub transport: Arc<dyn HttpTransport>,
}

/// Builds the provider for `arg`, reading the own key's settings through `env`, and the
/// transport: the recording when one is given.
pub fn setup(
    arg: ProviderArg,
    recording: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Setup, Failure> {
    let provider = match arg {
        ProviderArg::Fake => ResolvedProvider::fake(),
        ProviderArg::Local => ResolvedProvider::local(DEFAULT_LOCAL_BASE_URL, ""),
        ProviderArg::OwnKey => {
            let var = |name: &str| env(name).filter(|v| !v.trim().is_empty());
            match (var(OWN_KEY_VAR), var(OWN_BASE_URL_VAR), var(OWN_MODEL_VAR)) {
                (Some(key), Some(base_url), Some(model)) => ResolvedProvider::own_key(
                    base_url.trim(),
                    SecretString::from(key.trim()),
                    model.trim(),
                ),
                _ => {
                    return Err(Failure::usage(format!(
                        "--provider own-key needs {OWN_KEY_VAR}, {OWN_BASE_URL_VAR} and {OWN_MODEL_VAR}."
                    )));
                }
            }
        }
    };
    let transport: Arc<dyn HttpTransport> = match recording {
        Some(path) => Arc::new(
            FakeTransport::from_file(Path::new(path)).map_err(|e| Failure::usage(e.to_string()))?,
        ),
        None if provider.kind == ProviderKind::Fake => {
            Arc::new(FakeTransport::scripted(Vec::new()))
        }
        None => Arc::new(ReqwestTransport::new().map_err(|e| Failure::usage(e.to_string()))?),
    };
    Ok(Setup {
        provider,
        transport,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn provider_names_parse() {
        assert_eq!("fake".parse::<ProviderArg>().unwrap(), ProviderArg::Fake);
        assert_eq!("local".parse::<ProviderArg>().unwrap(), ProviderArg::Local);
        assert_eq!(
            "own-key".parse::<ProviderArg>().unwrap(),
            ProviderArg::OwnKey
        );
        assert_eq!(
            "publik".parse::<ProviderArg>().unwrap_err(),
            Failure::usage("Unknown provider \"publik\". Use local, own-key or fake.")
        );
    }

    #[test]
    fn the_fake_and_local_providers() {
        let fake = setup(ProviderArg::Fake, None, no_env).unwrap();
        assert_eq!(fake.provider.kind, ProviderKind::Fake);
        assert_eq!(fake.provider.base_url, "http://fake.invalid/v1");
        assert_eq!(fake.provider.chat_model, "fake-chat");
        let local = setup(ProviderArg::Local, None, no_env).unwrap();
        assert_eq!(local.provider.kind, ProviderKind::Local);
        assert_eq!(local.provider.base_url, "http://localhost:11434/v1");
        assert!(local.provider.chat_model.is_empty());
    }

    #[test]
    fn own_key_reads_its_three_variables() {
        let env = |name: &str| match name {
            "REDPEN_OWN_KEY" => Some("sk-test-a".to_string()),
            "REDPEN_OWN_BASE_URL" => Some("https://api.example.com/v1".to_string()),
            "REDPEN_OWN_MODEL" => Some("example-chat".to_string()),
            _ => None,
        };
        let own = setup(ProviderArg::OwnKey, None, env).unwrap();
        assert_eq!(own.provider.kind, ProviderKind::OwnKey);
        assert_eq!(own.provider.base_url, "https://api.example.com/v1");
        assert_eq!(own.provider.chat_model, "example-chat");
        assert!(!format!("{:?}", own.provider).contains("sk-test-a"));
        let failure = setup(ProviderArg::OwnKey, None, no_env).err().unwrap();
        assert_eq!(failure.code, crate::cmd::USAGE);
        assert_eq!(
            failure.message,
            "--provider own-key needs REDPEN_OWN_KEY, REDPEN_OWN_BASE_URL and REDPEN_OWN_MODEL."
        );
    }

    #[test]
    fn a_missing_recording_is_a_usage_error() {
        let failure = setup(ProviderArg::Fake, Some("no-such-recording.json"), no_env)
            .err()
            .unwrap();
        assert_eq!(failure.code, crate::cmd::USAGE);
        assert!(
            failure.message.contains("could not be read"),
            "{}",
            failure.message
        );
    }
}
