//! The provider-neutral request and answer types.

use std::fmt;
use std::str::FromStr;

use av_utils::error::{Error, Result};
use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    OpenAi,
    Mistral,
    Gemini,
    Ollama,
}

impl Provider {
    // ------------------------------------------------------------------------
    /// What the app knows of it (`PROVIDERS`).
    pub fn spec(self) -> &'static ProviderSpec {
        &PROVIDERS[self as usize]
    }

    // ------------------------------------------------------------------------
    /// Its name in account files, messages and logs (`ProviderSpec::key`).
    pub fn as_str(self) -> &'static str {
        self.spec().key
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Provider {
    type Err = Error;

    // ------------------------------------------------------------------------
    /// A provider's key or one of its aliases, in any case (`claude` is
    /// Anthropic).
    fn from_str(s: &str) -> Result<Self> {
        let s = s.to_ascii_lowercase();
        PROVIDERS
            .iter()
            .find(|spec| spec.key == s || spec.aliases.contains(&s.as_str()))
            .map(|spec| spec.provider)
            .ok_or(Error::InvalidArgument {
                arg: format!("unknown provider {s:?}"),
            })
    }
}

// ----------------------------------------------------------------------------
/// How a provider takes the API key.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Auth {
    /// `Authorization: Bearer <key>`.
    Bearer,
    /// The key as this header's value.
    Header(&'static str),
}

// ----------------------------------------------------------------------------
/// What the app knows of a provider, in one place (`architecture.md#crates`):
/// the account files, the Accounts page and every request read it here.
#[derive(Debug)]
pub struct ProviderSpec {
    pub provider: Provider,
    /// Its name in account files, messages and logs.
    pub key: &'static str,
    /// Other names an account file may give it.
    pub aliases: &'static [&'static str],
    /// The name the user reads.
    pub display: &'static str,
    /// Where the API lives, unless the account names another host (a
    /// remote Ollama, ollama.com).
    pub default_url: &'static str,
    /// Whether it takes an API key; one that doesn't is addressed instead
    /// (Ollama, on this computer or another).
    pub needs_key: bool,
    /// Whether it keeps uploaded files (`Client::upload`); else files go
    /// inline.
    pub uploads: bool,
    /// How the key is sent, if the account has one.
    pub(crate) auth: Auth,
    /// Headers every request carries.
    pub(crate) headers: &'static [(&'static str, &'static str)],
}

// ----------------------------------------------------------------------------
/// Every provider, in the order of `Provider`'s variants.
pub const PROVIDERS: [ProviderSpec; 5] = [
    ProviderSpec {
        provider: Provider::Anthropic,
        key: "anthropic",
        aliases: &["claude"],
        display: "Anthropic",
        default_url: "https://api.anthropic.com/v1",
        needs_key: true,
        uploads: true,
        auth: Auth::Header("x-api-key"),
        headers: &[("anthropic-version", "2023-06-01")],
    },
    ProviderSpec {
        provider: Provider::OpenAi,
        key: "openai",
        aliases: &[],
        display: "OpenAI",
        default_url: "https://api.openai.com/v1",
        needs_key: true,
        uploads: true,
        auth: Auth::Bearer,
        headers: &[],
    },
    ProviderSpec {
        provider: Provider::Mistral,
        key: "mistral",
        aliases: &[],
        display: "Mistral",
        default_url: "https://api.mistral.ai/v1",
        needs_key: true,
        uploads: true,
        auth: Auth::Bearer,
        headers: &[],
    },
    ProviderSpec {
        provider: Provider::Gemini,
        key: "gemini",
        aliases: &[],
        display: "Gemini",
        default_url: "https://generativelanguage.googleapis.com/v1beta",
        needs_key: true,
        uploads: true,
        auth: Auth::Header("x-goog-api-key"),
        headers: &[],
    },
    ProviderSpec {
        provider: Provider::Ollama,
        key: "ollama",
        aliases: &[],
        display: "Ollama",
        default_url: "http://localhost:11434",
        needs_key: false,
        uploads: false,
        auth: Auth::Bearer,
        headers: &[],
    },
];

// ----------------------------------------------------------------------------
/// An address as a base URL: `nas:11434` → `http://nas:11434`, without a
/// trailing slash.
pub fn base_url(address: &str) -> String {
    let address = address.trim().trim_end_matches('/');
    if address.contains("://") {
        address.to_string()
    } else {
        format!("http://{address}")
    }
}

// ----------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelInfo {
    /// What `ChatRequest::model` takes.
    pub id: String,
    /// For display.
    pub name: String,
    /// Input limit in tokens, where the provider tells.
    pub context_window: Option<u64>,
    /// Output limit in tokens, where the provider tells.
    pub max_output: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn every_provider_has_its_spec_and_is_found_by_its_names() {
        for (i, spec) in PROVIDERS.iter().enumerate() {
            assert_eq!(spec.provider as usize, i, "{} out of order", spec.key);
            assert_eq!(spec.provider.spec().key, spec.key);
            assert_eq!(spec.key.parse::<Provider>().unwrap(), spec.provider);
        }
        assert_eq!("Claude".parse::<Provider>().unwrap(), Provider::Anthropic);
        assert_eq!("OpenAI".parse::<Provider>().unwrap(), Provider::OpenAi);
        assert!("gpt".parse::<Provider>().is_err());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn addresses_get_a_scheme_and_lose_the_trailing_slash() {
        assert_eq!(base_url(" nas:11434/ "), "http://nas:11434");
        assert_eq!(base_url("https://ollama.com"), "https://ollama.com");
    }
}
