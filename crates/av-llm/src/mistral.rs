//! Mistral: Chat Completions and Models (docs.mistral.ai/api).

use std::io::Read;

use av_http::Method;
use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde_json::Value;
use serde_json::json;

use crate::client::data_url;
use crate::client::{CHAT_TIMEOUT, cut_short, open, stream_error};
use crate::client::{Client, count, limit, send_json, string};
use crate::stream::read_sse;
use crate::types::ModelInfo;
use crate::types::{ChatRequest, ChatResponse, Part, Role, StopReason};
use crate::usage::Usage;

// ----------------------------------------------------------------------------
/// The chat models (aliases like `mistral-large-latest` included), newest
/// first.
pub(crate) fn models(client: &Client, cancel: &Cancel) -> Result<Vec<ModelInfo>> {
    let list: Value = send_json(client.request(cancel, Method::Get, "/models"))?;
    let mut models: Vec<_> = list["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| model["capabilities"]["completion_chat"] == true)
        .map(|model| {
            let id = string(&model["id"]);
            let info = ModelInfo {
                name: id.clone(),
                id,
                context_window: limit(&model["max_context_length"]),
                max_output: None,
            };
            (count(&model["created"]), info)
        })
        .collect();
    models.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    Ok(models.into_iter().map(|(_, info)| info).collect())
}

// ----------------------------------------------------------------------------
pub(crate) fn chat_body(req: &ChatRequest) -> Result<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &req.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    for message in &req.messages {
        match message.role {
            Role::User => {
                let content = message
                    .parts
                    .iter()
                    .map(content_chunk)
                    .collect::<Result<Vec<_>>>()?;
                messages.push(json!({"role": "user", "content": content}));
            }
            // Text only (checked by the client).
            Role::Assistant => {
                let text: String = message
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                messages.push(json!({"role": "assistant", "content": text}));
            }
        }
    }
    let mut body = json!({"model": req.model, "messages": messages});
    body["stream"] = json!(true);
    Ok(body)
}

// ----------------------------------------------------------------------------
/// Images as `image_url`, documents as `document_url`: data URLs inline.
fn content_chunk(part: &Part) -> Result<Value> {
    let chunk = match part {
        Part::Text(text) => json!({"type": "text", "text": text}),
        Part::Inline(file) if file.is_image() => {
            json!({"type": "image_url", "image_url": data_url(file)})
        }
        Part::Inline(file) => json!({
            "type": "document_url", "document_url": data_url(file), "document_name": file.name,
        }),
    };
    Ok(chunk)
}

// ----------------------------------------------------------------------------
pub(crate) fn chat(
    client: &Client,
    cancel: &Cancel,
    _model: &str,
    body: Value,
    on_delta: &mut dyn FnMut(&str),
) -> Result<ChatResponse> {
    let req = client
        .request(cancel, Method::Post, "/chat/completions")
        .timeout(CHAT_TIMEOUT)
        .json(&body)?;
    read_stream(open(req)?, on_delta)
}

// ----------------------------------------------------------------------------
/// A message's content: a string, or chunks of which the text ones count
/// (reasoning models add `thinking` chunks).
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(chunks) => chunks
            .iter()
            .filter(|chunk| chunk["type"] == "text")
            .filter_map(|chunk| chunk["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}

// ----------------------------------------------------------------------------
fn stop_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::End,
        "length" | "model_length" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_string()),
    }
}

// ----------------------------------------------------------------------------
/// Cached tokens, where reported, are part of `prompt_tokens`.
fn usage(usage: &Value) -> Usage {
    let cached = count(&usage["prompt_tokens_details"]["cached_tokens"]);
    Usage {
        input: count(&usage["prompt_tokens"]).saturating_sub(cached),
        output: count(&usage["completion_tokens"]),
        cache_read: cached,
        cache_write: 0,
        reasoning: 0,
    }
}

// ----------------------------------------------------------------------------
/// Chunks with content deltas; the last one has the finish reason and the
/// usage; `[DONE]` ends the stream.
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
        if data == "[DONE]" {
            return Ok(());
        }
        let chunk: Value = serde_json::from_str(data)?;
        if let Some(err) = stream_error(&chunk) {
            return Err(err);
        }
        if resp.model.is_empty() {
            resp.model = string(&chunk["model"]);
        }
        let choice = &chunk["choices"][0];
        let text = content_text(&choice["delta"]["content"]);
        if !text.is_empty() {
            resp.text.push_str(&text);
            on_delta(&text);
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            resp.stop = stop_reason(reason);
            finished = true;
        }
        if chunk["usage"].is_object() {
            resp.usage = usage(&chunk["usage"]);
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
    fn chat_body_has_system_message_and_url_chunks() {
        let image = FileData {
            name: "b.png".into(),
            mime_type: "image/png".into(),
            data: vec![1, 2, 3],
        };
        let req = ChatRequest::new("mistral-medium-latest")
            .system("Be brief.")
            .message(Message::user("Look.").with(Part::Inline(image)));
        assert_eq!(
            chat_body(&req).unwrap(),
            json!({
                "model": "mistral-medium-latest",
                "stream": true,
                "messages": [
                    {"role": "system", "content": "Be brief."},
                    {"role": "user", "content": [
                        {"type": "text", "text": "Look."},
                        {"type": "image_url", "image_url": "data:image/png;base64,AQID"},
                    ]},
                ],
            })
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn stream_gives_deltas_and_usage_of_the_last_chunk() {
        let body = r#"data: {"id":"c1","object":"chat.completion.chunk","model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":"Bon"},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":"jour"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"total_tokens":8,"completion_tokens":3}}

data: [DONE]

"#;
        let mut deltas = Vec::new();
        let resp = read_stream(body.as_bytes(), &mut |d| deltas.push(d.to_string())).unwrap();
        assert_eq!(deltas, ["Bon", "jour"]);
        assert_eq!(resp.text, "Bonjour");
        assert_eq!(resp.model, "mistral-small-latest");
        assert_eq!(resp.stop, StopReason::End);
        assert_eq!((resp.usage.input, resp.usage.output), (5, 3));
    }
}
