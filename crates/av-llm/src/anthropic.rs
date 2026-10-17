//! Anthropic: Models (platform.claude.com/docs/en/api).

use av_http::Method;
use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde_json::Value;

use crate::client::{Client, limit, send_json, string};
use crate::types::ModelInfo;

// ----------------------------------------------------------------------------
/// Newest first, as the API lists them.
pub(crate) fn models(client: &Client, cancel: &Cancel) -> Result<Vec<ModelInfo>> {
    let mut models = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut req = client
            .request(cancel, Method::Get, "/models")
            .query(&[("limit", "1000")]);
        if let Some(after) = &after {
            req = req.query(&[("after_id", after)]);
        }
        let page: Value = send_json(req)?;
        for model in page["data"].as_array().into_iter().flatten() {
            let id = string(&model["id"]);
            models.push(ModelInfo {
                name: model["display_name"].as_str().unwrap_or(&id).to_string(),
                context_window: limit(&model["max_input_tokens"]),
                max_output: limit(&model["max_tokens"]),
                id,
            });
        }
        match page["last_id"].as_str() {
            Some(last) if page["has_more"] == true => after = Some(last.to_string()),
            _ => return Ok(models),
        }
    }
}
