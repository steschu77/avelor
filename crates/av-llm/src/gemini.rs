//! Gemini: generateContent and Models (ai.google.dev/gemini-api).

use std::io::Read;

use av_http::Method;
use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde_json::Value;
use serde_json::json;

use crate::client::{CHAT_TIMEOUT, count, cut_short, open, stream_error};
use crate::client::{Client, limit, send_json, string};
use crate::stream::read_sse;
use crate::types::ModelInfo;
use crate::types::{ChatRequest, ChatResponse, Part, Role, StopReason};
use crate::usage::Usage;

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

// ----------------------------------------------------------------------------
/// The model goes into the URL, streaming is the endpoint.
pub(crate) fn chat_body(req: &ChatRequest) -> Result<Value> {
    let mut contents = Vec::new();
    for message in &req.messages {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "model",
        };
        let parts = message
            .parts
            .iter()
            .map(content_part)
            .collect::<Result<Vec<_>>>()?;
        contents.push(json!({"role": role, "parts": parts}));
    }
    let mut body = json!({"contents": contents});
    if let Some(system) = &req.system {
        body["systemInstruction"] = json!({"parts": [{"text": system}]});
    }
    Ok(body)
}

// ----------------------------------------------------------------------------
fn content_part(part: &Part) -> Result<Value> {
    let part = match part {
        Part::Text(text) => json!({"text": text}),
        Part::Inline(file) => json!({"inlineData": {
            "mimeType": file.mime_type,
            "data": av_utils::base64::to_base64(&file.data),
        }}),
    };
    Ok(part)
}

// ----------------------------------------------------------------------------
pub(crate) fn chat(
    client: &Client,
    cancel: &Cancel,
    model: &str,
    body: Value,
    on_delta: &mut dyn FnMut(&str),
) -> Result<ChatResponse> {
    let model = model.strip_prefix("models/").unwrap_or(model);
    let url = client.url(&format!("/models/{model}:streamGenerateContent?alt=sse"));
    let req = client
        .request(cancel, Method::Post, &url)
        .timeout(CHAT_TIMEOUT)
        .json(&body)?;
    read_stream(open(req)?, on_delta)
}

// ----------------------------------------------------------------------------
/// What one answer, or one streamed piece of it, holds.
struct Chunk {
    text: String,
    model: String,
    /// Only on the last piece.
    stop: Option<StopReason>,
    usage: Option<Usage>,
}

// ----------------------------------------------------------------------------
fn parse_chunk(resp: &Value) -> Chunk {
    let candidate = &resp["candidates"][0];
    let text = candidate["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["thought"] != true)
        .filter_map(|part| part["text"].as_str())
        .collect();
    let stop = match candidate["finishReason"].as_str() {
        Some(reason) => Some(stop_reason(reason)),
        // A prompt blocked outright gets no candidate.
        None if resp["promptFeedback"]["blockReason"].is_string() => Some(StopReason::Refused),
        None => None,
    };
    Chunk {
        text,
        model: string(&resp["modelVersion"]),
        stop,
        usage: resp["usageMetadata"]
            .is_object()
            .then(|| usage(&resp["usageMetadata"])),
    }
}

// ----------------------------------------------------------------------------
fn stop_reason(reason: &str) -> StopReason {
    match reason {
        "STOP" => StopReason::End,
        "MAX_TOKENS" => StopReason::MaxTokens,
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY" => {
            StopReason::Refused
        }
        other => StopReason::Other(other.to_string()),
    }
}

// ----------------------------------------------------------------------------
/// Cached tokens are part of the prompt count; thoughts are counted apart
/// from the candidates, but billed as output.
fn usage(usage: &Value) -> Usage {
    let cached = count(&usage["cachedContentTokenCount"]);
    let thoughts = count(&usage["thoughtsTokenCount"]);
    Usage {
        input: count(&usage["promptTokenCount"]).saturating_sub(cached),
        output: count(&usage["candidatesTokenCount"]) + thoughts,
        cache_read: cached,
        cache_write: 0,
        reasoning: thoughts,
    }
}

// ----------------------------------------------------------------------------
/// Each event is a piece of the answer; the last one has the finish
/// reason, the usage comes with the pieces (the last count holds).
fn read_stream(body: impl Read, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
    let mut resp = ChatResponse {
        text: String::new(),
        thinking: String::new(),
        model: String::new(),
        stop: StopReason::Other(String::new()),
        usage: Usage::default(),
        cost: None,
    };
    let mut finished = false;
    read_sse(body, |_, data| {
        let json: Value = serde_json::from_str(data)?;
        if let Some(err) = stream_error(&json) {
            return Err(err);
        }
        let chunk = parse_chunk(&json);
        if !chunk.text.is_empty() {
            resp.text.push_str(&chunk.text);
            on_delta(&chunk.text);
        }
        if !chunk.model.is_empty() {
            resp.model = chunk.model;
        }
        if let Some(usage) = chunk.usage {
            resp.usage = usage;
        }
        if let Some(stop) = chunk.stop {
            resp.stop = stop;
            finished = true;
        }
        Ok(())
    })?;
    if !finished {
        return Err(cut_short());
    }
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileData;
    use crate::types::Message;

    // ------------------------------------------------------------------------
    #[test]
    fn chat_body_has_contents() {
        let req = ChatRequest::new("gemini-3.8-flash")
            .system("Be brief.")
            .message(Message::user("Read."))
            .message(Message::assistant("Read it."))
            .message(Message::user("Now this.").with(Part::Inline(FileData {
                name: "b.png".into(),
                mime_type: "image/png".into(),
                data: vec![1, 2, 3],
            })));
        assert_eq!(
            chat_body(&req).unwrap(),
            json!({
                "systemInstruction": {"parts": [{"text": "Be brief."}]},
                "contents": [
                    {"role": "user", "parts": [
                        {"text": "Read."},
                    ]},
                    {"role": "model", "parts": [{"text": "Read it."}]},
                    {"role": "user", "parts": [
                        {"text": "Now this."},
                        {"inlineData": {"mimeType": "image/png", "data": "AQID"}},
                    ]},
                ],
            })
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn stream_skips_thoughts_and_normalizes_usage() {
        let body = r#"data: {"candidates": [{"content": {"parts": [{"text": "Planning...", "thought": true}],"role": "model"},"index": 0}],"usageMetadata": {"promptTokenCount": 100,"totalTokenCount": 150,"thoughtsTokenCount": 50},"modelVersion": "gemini-3.8-flash"}

data: {"candidates": [{"content": {"parts": [{"text": "Hel"}],"role": "model"},"index": 0}],"usageMetadata": {"promptTokenCount": 100,"candidatesTokenCount": 1,"totalTokenCount": 151,"thoughtsTokenCount": 50},"modelVersion": "gemini-3.8-flash"}

data: {"candidates": [{"content": {"parts": [{"text": "lo"}],"role": "model"},"finishReason": "STOP","index": 0}],"usageMetadata": {"promptTokenCount": 100,"cachedContentTokenCount": 60,"candidatesTokenCount": 2,"totalTokenCount": 152,"thoughtsTokenCount": 50},"modelVersion": "gemini-3.8-flash"}

"#;
        let mut deltas = Vec::new();
        let resp = read_stream(body.as_bytes(), &mut |d| deltas.push(d.to_string())).unwrap();
        assert_eq!(deltas, ["Hel", "lo"]);
        assert_eq!(resp.text, "Hello");
        assert_eq!(resp.model, "gemini-3.8-flash");
        assert_eq!(resp.stop, StopReason::End);
        assert_eq!(
            resp.usage,
            Usage {
                input: 40,
                output: 52,
                cache_read: 60,
                cache_write: 0,
                reasoning: 50
            }
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn blocked_prompt_is_refused() {
        let resp = json!({"promptFeedback": {"blockReason": "SAFETY"},
            "usageMetadata": {"promptTokenCount": 8, "totalTokenCount": 8}});
        let chunk = parse_chunk(&resp);
        assert_eq!(chunk.stop, Some(StopReason::Refused));
        assert_eq!(chunk.text, "");
    }
}
