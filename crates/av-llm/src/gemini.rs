//! Gemini: Models (ai.google.dev/gemini-api).

use av_http::Method;
use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde_json::Value;

use crate::client::{Client, limit, send_json, string};
use crate::types::ModelInfo;

// ----------------------------------------------------------------------------
/// The models that generate content, as the API lists them.
pub(crate) fn models(client: &Client, cancel: &Cancel) -> Result<Vec<ModelInfo>> {
    let mut models = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut req = client
            .request(cancel, Method::Get, &client.url("/models"))
            .query(&[("pageSize", "1000")]);
        if let Some(token) = &page_token {
            req = req.query(&[("pageToken", token)]);
        }
        let page: Value = send_json(req)?;
        let chat_models = page["models"].as_array().into_iter().flatten().filter(|m| {
            m["supportedGenerationMethods"]
                .as_array()
                .is_some_and(|methods| methods.iter().any(|m| m == "generateContent"))
        });
        for model in chat_models {
            let name = string(&model["name"]);
            let id = name.strip_prefix("models/").unwrap_or(&name).to_string();
            models.push(ModelInfo {
                name: model["displayName"].as_str().unwrap_or(&id).to_string(),
                context_window: limit(&model["inputTokenLimit"]),
                max_output: limit(&model["outputTokenLimit"]),
                id,
            });
        }
        match page["nextPageToken"].as_str() {
            Some(token) if !token.is_empty() => page_token = Some(token.to_string()),
            _ => return Ok(models),
        }
    }
}
