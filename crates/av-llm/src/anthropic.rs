//! Anthropic: the Messages API and Models
//! (platform.claude.com/docs/en/api).

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
use crate::types::{ChatRequest, ChatResponse, Part, StopReason};
use crate::usage::Usage;

/// The API requires a limit; enough for a long answer.
const DEFAULT_MAX_TOKENS: u32 = 8192;

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

// ----------------------------------------------------------------------------
pub(crate) fn chat_body(req: &ChatRequest) -> Result<Value> {
    let mut messages = Vec::new();
    for message in &req.messages {
        let content = message
            .parts
            .iter()
            .map(content_block)
            .collect::<Result<Vec<_>>>()?;
        messages.push(json!({"role": message.role, "content": content}));
    }
    let mut body = json!({
        "model": req.model,
        "max_tokens": DEFAULT_MAX_TOKENS,
        "messages": messages,
    });
    if let Some(system) = &req.system {
        body["system"] = json!(system);
    }
    body["stream"] = json!(true);
    Ok(body)
}

// ----------------------------------------------------------------------------
fn content_block(part: &Part) -> Result<Value> {
    let block = match part {
        Part::Text(text) => json!({"type": "text", "text": text}),
    };
    Ok(block)
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
        .request(cancel, Method::Post, "/messages")
        .timeout(CHAT_TIMEOUT)
        .json(&body)?;
    read_stream(open(req)?, on_delta)
}

// ----------------------------------------------------------------------------
fn stop_reason(reason: &Value) -> StopReason {
    match reason.as_str().unwrap_or_default() {
        "end_turn" | "stop_sequence" => StopReason::End,
        "max_tokens" => StopReason::MaxTokens,
        "refusal" => StopReason::Refused,
        other => StopReason::Other(other.to_string()),
    }
}

// ----------------------------------------------------------------------------
/// Thinking is part of `output_tokens`; input counts leave the cache out.
fn usage(usage: &Value) -> Usage {
    Usage {
        input: count(&usage["input_tokens"]),
        output: count(&usage["output_tokens"]),
        cache_read: count(&usage["cache_read_input_tokens"]),
        cache_write: count(&usage["cache_creation_input_tokens"]),
        reasoning: 0,
    }
}

// ----------------------------------------------------------------------------
/// `message_start` brings the model and the input usage, text deltas the
/// answer, `message_delta` the stop reason and the final (cumulative)
/// counts, `message_stop` the end.
fn read_stream(body: impl Read, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
    let mut resp = ChatResponse {
        text: String::new(),
        thinking: String::new(),
        model: String::new(),
        stop: StopReason::Other(String::new()),
        usage: Usage::default(),
        cost: None,
    };
    let mut done = false;
    read_sse(body, |_, data| {
        let event: Value = serde_json::from_str(data)?;
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                resp.model = string(&event["message"]["model"]);
                resp.usage = usage(&event["message"]["usage"]);
            }
            "content_block_delta" if event["delta"]["type"] == "text_delta" => {
                let text = event["delta"]["text"].as_str().unwrap_or_default();
                if !text.is_empty() {
                    resp.text.push_str(text);
                    on_delta(text);
                }
            }
            "message_delta" => {
                resp.stop = stop_reason(&event["delta"]["stop_reason"]);
                let counts = &event["usage"];
                let fields = [
                    ("output_tokens", &mut resp.usage.output),
                    ("input_tokens", &mut resp.usage.input),
                    ("cache_read_input_tokens", &mut resp.usage.cache_read),
                    ("cache_creation_input_tokens", &mut resp.usage.cache_write),
                ];
                for (name, field) in fields {
                    if let Some(n) = counts[name].as_u64() {
                        *field = n;
                    }
                }
            }
            "message_stop" => done = true,
            "error" => return Err(stream_error(&event).unwrap_or_else(cut_short)),
            _ => {}
        }
        Ok(())
    })?;
    if !done {
        return Err(cut_short());
    }
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use av_utils::error::Error;

    // ------------------------------------------------------------------------
    #[test]
    fn chat_body_has_blocks_per_part() {
        let req = ChatRequest::new("claude-sonnet-5-5")
            .system("Be brief.")
            .message(Message::user("Compare."))
            .message(Message::assistant("Done."));
        assert_eq!(
            chat_body(&req).unwrap(),
            json!({
                "model": "claude-sonnet-5-5",
                "max_tokens": 8192,
                "system": "Be brief.",
                "stream": true,
                "messages": [
                    {"role": "user", "content": [
                        {"type": "text", "text": "Compare."},
                    ]},
                    {"role": "assistant", "content": [{"type": "text", "text": "Done."}]},
                ],
            })
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn stream_gives_deltas_and_final_counts() {
        let body = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-haiku-4-5-20251001","content":[],"stop_reason":null,"usage":{"input_tokens":25,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}

event: ping
data: {"type":"ping"}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" you"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":15}}

event: message_stop
data: {"type":"message_stop"}

"#;
        let mut deltas = Vec::new();
        let resp = read_stream(body.as_bytes(), &mut |d| deltas.push(d.to_string())).unwrap();
        assert_eq!(deltas, ["Hi", " you"]);
        assert_eq!(resp.text, "Hi you");
        assert_eq!(resp.model, "claude-haiku-4-5-20251001");
        assert_eq!(resp.stop, StopReason::End);
        assert_eq!((resp.usage.input, resp.usage.output), (25, 15));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn stream_errors_and_cut_offs_fail() {
        let overloaded = "event: error\ndata: {\"type\":\"error\",\"error\":\
                          {\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        match read_stream(overloaded.as_bytes(), &mut |_| {}) {
            Err(Error::InvalidResponse { msg }) => assert_eq!(msg, "overloaded_error: Overloaded"),
            other => panic!("unexpected {other:?}"),
        }
        let cut = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\
                   \"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n";
        assert!(matches!(
            read_stream(cut.as_bytes(), &mut |_| {}),
            Err(Error::InvalidResponse { .. })
        ));
    }
}
