//! `--provider local|own-key|publik-env|fake` (docs/modules/02-provider-layer.md,
//! docs/ARCHITECTURE.md §5): the provider a command calls, and the transport it calls it through.
//!
//! - `fake`: the fake provider (`http://fake.invalid/v1`, model `fake-chat`, no key). A rewrite
//!   on it sends the publik tier as its model, so a recording holds the publik API body.
//! - `own-key`: the user's own key from `REDPEN_OWN_KEY`, `REDPEN_OWN_BASE_URL` and
//!   `REDPEN_OWN_MODEL`; the key is read into a secret and never printed.
//! - `local`: Ollama at `http://localhost:11434/v1`, with the model Ollama's list gives.
//! - `publik-env` (module 04): publik API with the key in `PUBLIK_API_KEY`, at
//!   `PUBLIK_API_BASE_URL` or publik's base URL. The command line never provisions publik API
//!   and never reads the app's credential file.
//!
//! With `--recording R`, every provider answers from that `recording.json` through the fake
//! transport, so nothing reaches the network; without it, `fake` answers nothing (every request
//! is unrecorded) and the others use the network.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use redpen_core::net::{FakeTransport, HttpTransport, ReqwestTransport};
use redpen_core::provider::local::DEFAULT_LOCAL_BASE_URL;
use redpen_core::provider::tier::PUBLIK_BALANCED;
use redpen_core::provider::{Feature, ProviderKind, ResolvedProvider, SecretString, StatusSink};

use crate::cmd::Failure;

/// The environment variables of `--provider own-key`.
pub const OWN_KEY_VAR: &str = "REDPEN_OWN_KEY";
pub const OWN_BASE_URL_VAR: &str = "REDPEN_OWN_BASE_URL";
pub const OWN_MODEL_VAR: &str = "REDPEN_OWN_MODEL";

/// The environment variables of `--provider publik-env` (CONTRIBUTING.md, `## publik API`).
pub const PUBLIK_KEY_VAR: &str = "PUBLIK_API_KEY";
pub const PUBLIK_BASE_URL_VAR: &str = "PUBLIK_API_BASE_URL";
/// publik API's base URL when `PUBLIK_API_BASE_URL` is not set.
pub const PUBLIK_BASE_URL: &str = "https://publikhq.com/api/v1";

/// A `--provider` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderArg {
    Fake,
    Local,
    OwnKey,
    PublikEnv,
}

impl ProviderArg {
    /// The name as typed after `--provider`.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderArg::Fake => "fake",
            ProviderArg::Local => "local",
            ProviderArg::OwnKey => "own-key",
            ProviderArg::PublikEnv => "publik-env",
        }
    }
}

impl FromStr for ProviderArg {
    type Err = Failure;

    fn from_str(s: &str) -> Result<Self, Failure> {
        match s {
            "fake" => Ok(ProviderArg::Fake),
            "local" => Ok(ProviderArg::Local),
            "own-key" => Ok(ProviderArg::OwnKey),
            "publik-env" => Ok(ProviderArg::PublikEnv),
            other => Err(Failure::usage(format!(
                "Unknown provider \"{other}\". Use local, own-key, publik-env or fake."
            ))),
        }
    }
}

/// The provider and the transport a command uses.
pub struct Setup {
    pub provider: ResolvedProvider,
    pub transport: Arc<dyn HttpTransport>,
}

/// Builds the provider for `arg`, reading the own key's and publik API's settings through `env`,
/// and the transport: the recording when one is given.
pub fn setup(
    arg: ProviderArg,
    recording: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Setup, Failure> {
    let var = |name: &str| env(name).filter(|v| !v.trim().is_empty());
    let provider = match arg {
        ProviderArg::Fake => ResolvedProvider::fake(),
        ProviderArg::Local => ResolvedProvider::local(DEFAULT_LOCAL_BASE_URL, ""),
        ProviderArg::OwnKey => {
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
        ProviderArg::PublikEnv => {
            let Some(key) = var(PUBLIK_KEY_VAR) else {
                return Err(Failure::usage(format!(
                    "--provider publik-env needs {PUBLIK_KEY_VAR}."
                )));
            };
            let base_url = var(PUBLIK_BASE_URL_VAR).unwrap_or_else(|| PUBLIK_BASE_URL.to_string());
            ResolvedProvider {
                kind: ProviderKind::Publik,
                base_url: base_url.trim().to_string(),
                key: Some(SecretString::from(key.trim())),
                // As the app's resolver gives it; each call sends its own tier.
                chat_model: PUBLIK_BALANCED.to_string(),
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

/// Reads what a call cost from `x-publik-charge-micros` (whole micro-dollars), on publik API and
/// on the fake provider that stands in for it in recordings. Every other header is ignored.
#[derive(Debug, Default, Clone, Copy)]
pub struct ChargeSink;

impl StatusSink for ChargeSink {
    fn on_response(
        &self,
        _kind: ProviderKind,
        _feature: Feature,
        _status: u16,
        headers: &[(String, String)],
    ) -> Option<u64> {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("x-publik-charge-micros"))
            .and_then(|(_, value)| value.trim().parse().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::provider::ExposeSecret;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn provider_names_parse() {
        for arg in [
            ProviderArg::Fake,
            ProviderArg::Local,
            ProviderArg::OwnKey,
            ProviderArg::PublikEnv,
        ] {
            assert_eq!(arg.as_str().parse::<ProviderArg>().unwrap(), arg);
        }
        assert_eq!(
            "publik".parse::<ProviderArg>().unwrap_err(),
            Failure::usage("Unknown provider \"publik\". Use local, own-key, publik-env or fake.")
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
    fn publik_env_reads_the_key_and_the_base_url() {
        let env = |name: &str| match name {
            "PUBLIK_API_KEY" => Some("pk_test_a".to_string()),
            _ => None,
        };
        let publik = setup(ProviderArg::PublikEnv, None, env).unwrap();
        assert_eq!(publik.provider.kind, ProviderKind::Publik);
        assert_eq!(publik.provider.base_url, "https://publikhq.com/api/v1");
        assert_eq!(publik.provider.chat_model, "publik-balanced");
        assert_eq!(
            publik.provider.key.as_ref().unwrap().expose_secret(),
            "pk_test_a"
        );
        assert!(!format!("{:?}", publik.provider).contains("pk_test_a"));
        let elsewhere = |name: &str| match name {
            "PUBLIK_API_KEY" => Some("pk_test_a".to_string()),
            "PUBLIK_API_BASE_URL" => Some("http://localhost:8787/api/v1".to_string()),
            _ => None,
        };
        assert_eq!(
            setup(ProviderArg::PublikEnv, None, elsewhere)
                .unwrap()
                .provider
                .base_url,
            "http://localhost:8787/api/v1"
        );
        assert_eq!(
            setup(ProviderArg::PublikEnv, None, no_env)
                .err()
                .unwrap()
                .message,
            "--provider publik-env needs PUBLIK_API_KEY."
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

    #[test]
    fn the_charge_is_read_from_its_header() {
        let headers = |v: &str| vec![("X-Publik-Charge-Micros".to_string(), v.to_string())];
        let sink = ChargeSink;
        let read =
            |h: &[(String, String)]| sink.on_response(ProviderKind::Fake, Feature::Clearer, 200, h);
        assert_eq!(read(&headers("100")), Some(100));
        assert_eq!(read(&headers("x")), None);
        assert_eq!(read(&[]), None);
    }
}
