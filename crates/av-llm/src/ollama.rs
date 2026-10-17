//! Ollama's native API (docs.ollama.com/api), on this computer or at
//! ollama.com (with an API key).

use av_http::Method;
use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde_json::Value;

use crate::client::{Client, send_json, string};
use crate::types::ModelInfo;

// ----------------------------------------------------------------------------
/// The pulled models, by name.
pub(crate) fn models(client: &Client, cancel: &Cancel) -> Result<Vec<ModelInfo>> {
    let tags: Value = send_json(client.request(cancel, Method::Get, "/api/tags"))?;
    let mut models: Vec<ModelInfo> = tags["models"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|model| {
            let id = string(&model["name"]);
            let details = [
                &model["details"]["parameter_size"],
                &model["details"]["quantization_level"],
            ]
            .into_iter()
            .filter_map(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
            let name = if details.is_empty() {
                id.clone()
            } else {
                format!("{id} ({})", details.join(", "))
            };
            ModelInfo {
                id,
                name,
                context_window: None,
                max_output: None,
            }
        })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}
