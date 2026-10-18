//! Ollama's native API (docs.ollama.com/api), on this computer or at
//! ollama.com (with an API key). No file store: images go inline.

use std::io::Read;

use av_http::Method;
use av_utils::base64::to_base64;
use av_utils::cancel::Cancel;
use av_utils::error::Error;
use av_utils::error::Result;
use serde_json::Value;
use serde_json::json;

use crate::client::{CHAT_TIMEOUT, count, cut_short, open, stream_error};
use crate::client::{Client, send_json, string};
use crate::stream::read_lines;
use crate::types::FileData;
use crate::types::ModelInfo;
use crate::types::{ChatRequest, ChatResponse, Part, StopReason};
use crate::usage::Usage;

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

// ----------------------------------------------------------------------------
pub(crate) fn chat_body(req: &ChatRequest) -> Result<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &req.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    for message in &req.messages {
        let mut texts = Vec::new();
        let mut images = Vec::new();
        for part in &message.parts {
            match part {
                Part::Text(text) => texts.push(text.clone()),
                Part::Inline(file) if file.is_image() => images.push(to_base64(&file.data)),
                Part::Inline(FileData { mime_type, .. }) => {
                    return Err(Error::Unsupported {
                        what: format!("{mime_type} files on Ollama"),
                    });
                }
            }
        }
        let mut entry = json!({"role": message.role, "content": texts.join("\n\n")});
        if !images.is_empty() {
            entry["images"] = json!(images);
        }
        messages.push(entry);
    }
    let mut body = json!({"model": req.model, "messages": messages, "stream": true});
    if let Some(think) = req.think {
        body["think"] = json!(think);
    }
    let mut options = json!({});
    if let Some(context) = req.context {
        options["num_ctx"] = json!(context);
    }
    if options.as_object().is_some_and(|o| !o.is_empty()) {
        body["options"] = options;
    }
    Ok(body)
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
        .request(cancel, Method::Post, "/api/chat")
        .timeout(CHAT_TIMEOUT)
        .json(&body)?;
    read_stream(open(req)?, on_delta)
}

// ----------------------------------------------------------------------------
fn stop_reason(reason: &Value) -> StopReason {
    match reason.as_str().unwrap_or_default() {
        "stop" => StopReason::End,
        "length" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_string()),
    }
}

// ----------------------------------------------------------------------------
/// `eval_count` includes thinking, when the model thinks.
fn usage(answer: &Value) -> Usage {
    Usage {
        input: count(&answer["prompt_eval_count"]),
        output: count(&answer["eval_count"]),
        ..Usage::default()
    }
}

// ----------------------------------------------------------------------------
/// One JSON object per line, the last with `done` and the counts. A
/// thinking model's reasoning comes apart, as `thinking` (with `think`
/// on, or by default in newer Ollama versions), or in the text (older).
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
    read_lines(body, |line| {
        if line.trim().is_empty() {
            return Ok(());
        }
        let chunk: Value = serde_json::from_str(line)?;
        if let Some(err) = stream_error(&chunk) {
            return Err(err);
        }
        let thinking = chunk["message"]["thinking"].as_str().unwrap_or_default();
        resp.thinking.push_str(thinking);
        let text = chunk["message"]["content"].as_str().unwrap_or_default();
        if !text.is_empty() {
            resp.text.push_str(text);
            on_delta(text);
        }
        if chunk["done"] == true {
            resp.model = string(&chunk["model"]);
            resp.stop = stop_reason(&chunk["done_reason"]);
            resp.usage = usage(&chunk);
            done = true;
        }
        Ok(())
    })?;
    if !done {
        return Err(cut_short());
    }
    let (thinking, text) = split_thinking(&resp.text);
    resp.thinking.push_str(&thinking);
    resp.text = text;
    Ok(resp)
}

// ----------------------------------------------------------------------------
/// `text` split into the reasoning a thinking model (qwen3, deepseek-r1)
/// wrote into it and the answer: the answer is what follows its last
/// `</think>`, else all of it. The chat templates open the `<think>`
/// before the answer starts, so often only the closing tag shows. The
/// streamed deltas still carry the reasoning.
fn split_thinking(text: &str) -> (String, String) {
    const START: &str = "<think>";
    const END: &str = "</think>";
    match text.rfind(END) {
        Some(end) => {
            let reasoning = text[..end].trim_start();
            let reasoning = reasoning.strip_prefix(START).unwrap_or(reasoning);
            let answer = text[end + END.len()..].trim_start();
            (reasoning.trim().to_string(), answer.to_string())
        }
        None => (String::new(), text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    // ------------------------------------------------------------------------
    #[test]
    fn chat_body_inlines_images() {
        let req = ChatRequest::new("gemma3").system("Be brief.").message(
            Message::user("Describe.").with(Part::Inline(FileData {
                name: "a.png".into(),
                mime_type: "image/png".into(),
                data: vec![1, 2, 3],
            })),
        );
        assert_eq!(
            chat_body(&req).unwrap(),
            json!({
                "model": "gemma3",
                "stream": true,
                "messages": [
                    {"role": "system", "content": "Be brief."},
                    {"role": "user", "content": "Describe.", "images": ["AQID"]},
                ],
            })
        );

        let pdf =
            ChatRequest::new("gemma3").message(Message::user("?").with(Part::Inline(FileData {
                name: "a.pdf".into(),
                mime_type: "application/pdf".into(),
                data: vec![],
            })));
        assert!(matches!(chat_body(&pdf), Err(Error::Unsupported { .. })));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn stream_of_json_lines() {
        let body = concat!(
            r#"{"model":"llama3.2","created_at":"2026-09-29T10:00:00Z","message":{"role":"assistant","content":"Hi"},"done":false}"#,
            "\n",
            r#"{"model":"llama3.2","created_at":"2026-09-29T10:00:00Z","message":{"role":"assistant","content":"!"},"done":false}"#,
            "\n",
            r#"{"model":"llama3.2","created_at":"2026-09-29T10:00:01Z","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","total_duration":1,"prompt_eval_count":26,"eval_count":2}"#,
            "\n",
        );
        let mut deltas = Vec::new();
        let resp = read_stream(body.as_bytes(), &mut |d| deltas.push(d.to_string())).unwrap();
        assert_eq!(deltas, ["Hi", "!"]);
        assert_eq!(resp.text, "Hi!");
        assert_eq!(resp.model, "llama3.2");
        assert_eq!(resp.stop, StopReason::End);
        assert_eq!((resp.usage.input, resp.usage.output), (26, 2));

        let failed = "{\"error\":\"model runner has unexpectedly stopped\"}\n";
        match read_stream(failed.as_bytes(), &mut |_| {}) {
            Err(Error::InvalidResponse { msg }) => {
                assert_eq!(msg, "model runner has unexpectedly stopped")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_thinking_models_reasoning_is_left_out() {
        let line = |content: &str, done: bool| {
            let chunk = json!({"model": "qwen3", "message": {"content": content}, "done": done});
            format!("{chunk}\n")
        };
        let body = [
            line("Let me read the page.\nDraft", false),
            line("</th", false),
            line("ink>\n\n# Bill", false),
            line("", true),
        ]
        .concat();
        let resp = read_stream(body.as_bytes(), &mut |_| {}).unwrap();
        assert_eq!(resp.text, "# Bill");
        assert_eq!(resp.thinking, "Let me read the page.\nDraft");
        assert_eq!(
            split_thinking("<think>\nHmm.\n</think>\nAnswer"),
            ("Hmm.".to_string(), "Answer".to_string())
        );
        assert_eq!(
            split_thinking("No tags at all"),
            (String::new(), "No tags at all".to_string())
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn reasoning_apart_is_kept_apart() {
        let line = |thinking: &str, content: &str, done: bool| {
            let message = json!({"thinking": thinking, "content": content});
            let chunk = json!({"model": "qwen3", "message": message, "done": done, "done_reason": "length"});
            format!("{chunk}\n")
        };
        let body = [
            line("The mail ", "", false),
            line("is a bill.", "", false),
            line("", "", true),
        ]
        .concat();
        let mut deltas = Vec::new();
        let resp = read_stream(body.as_bytes(), &mut |d| deltas.push(d.to_string())).unwrap();
        assert_eq!(resp.thinking, "The mail is a bill.");
        assert_eq!(resp.text, "", "cut off while reasoning");
        assert_eq!(resp.stop, StopReason::MaxTokens);
        assert!(deltas.is_empty(), "the deltas are the answer's");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn think_and_the_context_go_to_ollama() {
        let req = ChatRequest::new("qwen3").think(false).context(8192);
        let body = chat_body(&req).unwrap();
        assert_eq!(body["think"], json!(false));
        assert_eq!(body["options"], json!({"num_ctx": 8192}));
        let plain = chat_body(&ChatRequest::new("qwen3")).unwrap();
        assert!(plain.get("think").is_none() && plain.get("options").is_none());
    }
}
