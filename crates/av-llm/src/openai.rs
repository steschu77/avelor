//! OpenAI: Models (developers.openai.com/api/docs).

use av_http::Method;
use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde_json::Value;

use crate::client::{Client, count, send_json, string};
use crate::types::ModelInfo;

// ----------------------------------------------------------------------------
/// Every model of the account (the API doesn't tell which ones chat),
/// newest first.
pub(crate) fn models(client: &Client, cancel: &Cancel) -> Result<Vec<ModelInfo>> {
    let list: Value = send_json(client.request(cancel, Method::Get, "/models"))?;
    let mut models: Vec<_> = list["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|model| {
            let id = string(&model["id"]);
            let info = ModelInfo {
                name: id.clone(),
                id,
                context_window: None,
                max_output: None,
            };
            (count(&model["created"]), info)
        })
        .collect();
    models.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    Ok(models.into_iter().map(|(_, info)| info).collect())
}
