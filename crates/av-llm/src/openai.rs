//! OpenAI: the Responses API and Models
//! (developers.openai.com/api/docs). Nothing is stored at OpenAI
//! (`store: false`); the conversation is sent in full every turn, as with
//! every other provider.

use std::io::Read;

use av_http::Method;
use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde_json::Value;
use serde_json::json;

use crate::client::{CHAT_TIMEOUT, cut_short, open, stream_error};
use crate::client::{Client, count, send_json, string};
use crate::stream::read_sse;
use crate::types::ModelInfo;
use crate::types::{ChatRequest, ChatResponse, Part, Role, StopReason};
use crate::usage::Usage;

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

// ----------------------------------------------------------------------------
pub(crate) fn chat_body(req: &ChatRequest) -> Result<Value> {
    let input: Vec<Value> = req
        .messages
        .iter()
        .map(|message| match message.role {
            Role::User => {
                let content: Vec<Value> = message.parts.iter().map(input_part).collect();
                json!({"role": "user", "content": content})
            }
            // Text only.
            Role::Assistant => {
                let text: String = message
                    .parts
                    .iter()
                    .map(|part| match part {
                        Part::Text(text) => text.as_str(),
                    })
                    .collect();
                json!({"role": "assistant", "content": text})
            }
        })
        .collect();
    let mut body = json!({"model": req.model, "input": input, "store": false});
    if let Some(system) = &req.system {
        body["instructions"] = json!(system);
    }
    body["stream"] = json!(true);
    Ok(body)
}

// ----------------------------------------------------------------------------
fn input_part(part: &Part) -> Value {
    match part {
        Part::Text(text) => json!({"type": "input_text", "text": text}),
    }
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
        .request(cancel, Method::Post, "/responses")
        .timeout(CHAT_TIMEOUT)
        .json(&body)?;
    read_stream(open(req)?, on_delta)
}

// ----------------------------------------------------------------------------
/// A response object: its message text (reasoning items left out), why it
/// stopped, and its usage.
fn parse_response(resp: &Value) -> Result<ChatResponse> {
    if resp["status"] == "failed" {
        return Err(stream_error(resp).unwrap_or_else(cut_short));
    }
    let mut text = String::new();
    let mut refused = false;
    let messages = resp["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "message");
    for content in messages.flat_map(|m| m["content"].as_array().into_iter().flatten()) {
        match content["type"].as_str().unwrap_or_default() {
            "output_text" => text.push_str(content["text"].as_str().unwrap_or_default()),
            "refusal" => {
                text.push_str(content["refusal"].as_str().unwrap_or_default());
                refused = true;
            }
            _ => {}
        }
    }
    let stop = match resp["status"].as_str().unwrap_or_default() {
        _ if refused => StopReason::Refused,
        "completed" => StopReason::End,
        "incomplete" => match resp["incomplete_details"]["reason"].as_str() {
            Some("max_output_tokens") => StopReason::MaxTokens,
            Some("content_filter") => StopReason::Refused,
            other => StopReason::Other(other.unwrap_or("incomplete").to_string()),
        },
        other => StopReason::Other(other.to_string()),
    };
    Ok(ChatResponse {
        text,
        thinking: String::new(),
        model: string(&resp["model"]),
        stop,
        usage: usage(&resp["usage"]),
        cost: None,
    })
}

// ----------------------------------------------------------------------------
/// Cached tokens are part of `input_tokens`, reasoning of `output_tokens`.
fn usage(usage: &Value) -> Usage {
    let cached = count(&usage["input_tokens_details"]["cached_tokens"]);
    Usage {
        input: count(&usage["input_tokens"]).saturating_sub(cached),
        output: count(&usage["output_tokens"]),
        cache_read: cached,
        cache_write: 0,
        reasoning: count(&usage["output_tokens_details"]["reasoning_tokens"]),
    }
}

// ----------------------------------------------------------------------------
/// Text deltas as they come; the final `response.completed` (or
/// `.incomplete`) event carries the whole response.
fn read_stream(body: impl Read, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
    let mut last = None;
    read_sse(body, |_, data| {
        if data == "[DONE]" {
            return Ok(());
        }
        let event: Value = serde_json::from_str(data)?;
        match event["type"].as_str().unwrap_or_default() {
            "response.output_text.delta" | "response.refusal.delta" => {
                let delta = event["delta"].as_str().unwrap_or_default();
                if !delta.is_empty() {
                    on_delta(delta);
                }
            }
            "response.completed" | "response.incomplete" => {
                last = Some(parse_response(&event["response"])?);
            }
            "response.failed" => {
                return Err(stream_error(&event["response"]).unwrap_or_else(cut_short));
            }
            "error" => return Err(stream_error(&event).unwrap_or_else(cut_short)),
            _ => {}
        }
        Ok(())
    })?;
    last.ok_or_else(cut_short)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use av_utils::error::Error;

    // ------------------------------------------------------------------------
    #[test]
    fn chat_body_has_input_items() {
        let req = ChatRequest::new("gpt-6-sol")
            .system("Be brief.")
            .message(Message::user("Hi"))
            .message(Message::assistant("Hello."))
            .message(Message::user("And this?"));
        assert_eq!(
            chat_body(&req).unwrap(),
            json!({
                "model": "gpt-6-sol",
                "store": false,
                "instructions": "Be brief.",
                "stream": true,
                "input": [
                    {"role": "user", "content": [
                        {"type": "input_text", "text": "Hi"},
                    ]},
                    {"role": "assistant", "content": "Hello."},
                    {"role": "user", "content": [
                        {"type": "input_text", "text": "And this?"},
                    ]},
                ],
            })
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn response_gives_text_stop_and_normalized_usage() {
        let resp = json!({
            "id": "resp_1", "object": "response", "status": "incomplete",
            "incomplete_details": {"reason": "max_output_tokens"},
            "model": "gpt-6-sol-2026-07-01",
            "output": [
                {"type": "reasoning", "id": "rs_1", "summary": []},
                {"type": "message", "role": "assistant", "content": [
                    {"type": "output_text", "text": "Part", "annotations": []},
                ]},
            ],
            "usage": {"input_tokens": 1000, "input_tokens_details": {"cached_tokens": 800},
                "output_tokens": 300, "output_tokens_details": {"reasoning_tokens": 250},
                "total_tokens": 1300},
        });
        let resp = parse_response(&resp).unwrap();
        assert_eq!(resp.text, "Part");
        assert_eq!(resp.model, "gpt-6-sol-2026-07-01");
        assert_eq!(resp.stop, StopReason::MaxTokens);
        assert_eq!(
            resp.usage,
            Usage {
                input: 200,
                output: 300,
                cache_read: 800,
                cache_write: 0,
                reasoning: 250
            }
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn stream_gives_deltas_and_the_completed_response() {
        let body = r#"event: response.created
data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_1","status":"in_progress","output":[]}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","sequence_number":4,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hi"}

event: response.output_text.delta
data: {"type":"response.output_text.delta","sequence_number":5,"item_id":"msg_1","output_index":0,"content_index":0,"delta":" there"}

event: response.completed
data: {"type":"response.completed","sequence_number":9,"response":{"id":"resp_1","status":"completed","model":"gpt-6-luna","output":[{"type":"message","content":[{"type":"output_text","text":"Hi there"}]}],"usage":{"input_tokens":9,"output_tokens":2}}}

"#;
        let mut deltas = Vec::new();
        let resp = read_stream(body.as_bytes(), &mut |d| deltas.push(d.to_string())).unwrap();
        assert_eq!(deltas, ["Hi", " there"]);
        assert_eq!(resp.text, "Hi there");
        assert_eq!(resp.stop, StopReason::End);
        assert_eq!((resp.usage.input, resp.usage.output), (9, 2));

        let failed = "data: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\",\
                      \"error\":{\"code\":\"server_error\",\"message\":\"Oops.\"}}}\n\n";
        match read_stream(failed.as_bytes(), &mut |_| {}) {
            Err(Error::InvalidResponse { msg }) => assert_eq!(msg, "server_error: Oops."),
            other => panic!("unexpected {other:?}"),
        }
    }
}
