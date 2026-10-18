//! The provider-neutral request and answer types.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use av_utils::error::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::usage::Usage;

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
    /// Anthropic, the account kind in accounts.md#ai-account-file).
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

// ----------------------------------------------------------------------------
/// One turn of the conversation. Files belong in user turns only.
#[derive(Clone, Debug)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    // ------------------------------------------------------------------------
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            parts: vec![Part::Text(text.into())],
        }
    }

    // ------------------------------------------------------------------------
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            parts: vec![Part::Text(text.into())],
        }
    }

    // ------------------------------------------------------------------------
    pub fn with(mut self, part: Part) -> Self {
        self.parts.push(part);
        self
    }
}

// ----------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub enum Part {
    Text(String),
    /// A file sent along with the request.
    Inline(FileData),
}

// ----------------------------------------------------------------------------
/// A file's name, type, and content: to send inline.
#[derive(Clone, Debug)]
pub struct FileData {
    pub name: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

impl FileData {
    // ------------------------------------------------------------------------
    /// Read `path`, its type guessed from the extension.
    pub fn from_path(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).map_err(|_| Error::FileRead {
            path: path.to_path_buf(),
        })?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Self {
            name,
            mime_type: av_utils::mime::type_for(path).to_string(),
            data,
        })
    }

    // ------------------------------------------------------------------------
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }
}

// ----------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    /// Whether a model that can reason does so before it answers; `None`:
    /// the model's default. Ollama only (`think`).
    pub think: Option<bool>,
    /// The context window to run the model with, in tokens; `None`: the
    /// server's default. Ollama only (`num_ctx`): the cloud providers'
    /// windows are fixed.
    pub context: Option<u32>,
}

impl ChatRequest {
    // ------------------------------------------------------------------------
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system: None,
            messages: Vec::new(),
            think: None,
            context: None,
        }
    }

    // ------------------------------------------------------------------------
    pub fn system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    // ------------------------------------------------------------------------
    pub fn message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    // ------------------------------------------------------------------------
    pub fn think(mut self, think: bool) -> Self {
        self.think = Some(think);
        self
    }

    // ------------------------------------------------------------------------
    pub fn context(mut self, tokens: u32) -> Self {
        self.context = Some(tokens);
        self
    }
}

// ----------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct ChatResponse {
    /// The answer's text (reasoning and thinking left out).
    pub text: String,
    /// The model's reasoning, where the provider hands it out (Ollama: its
    /// `thinking`, or what a model wrote before `</think>`); empty
    /// otherwise.
    pub thinking: String,
    /// The model that answered, as the provider names it (may be a dated
    /// version of the requested one).
    pub model: String,
    pub stop: StopReason,
    pub usage: Usage,
    /// USD; `None` if the model's price is unknown.
    pub cost: Option<f64>,
}

// ----------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The model finished its answer.
    End,
    /// The answer hit `max_tokens`.
    MaxTokens,
    /// The model or the provider's filter declined.
    Refused,
    /// Anything else, as the provider named it.
    Other(String),
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
