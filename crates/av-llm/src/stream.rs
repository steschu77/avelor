//! Reading streamed answers: server-sent events (Anthropic, OpenAI,
//! Mistral, Gemini) and newline-delimited JSON (Ollama).

use std::io::{BufRead, BufReader, Read};

use av_utils::error::Result;

use crate::client::io_error;

// ----------------------------------------------------------------------------
/// Call `on_event(event, data)` for each server-sent event in `body` until it
/// ends: `event` is the `event:` field (empty if none), `data` the `data:`
/// lines joined by `\n`. Comments and other fields are skipped; an event
/// the body ends in without a blank line is still delivered.
pub(crate) fn read_sse(
    body: impl Read,
    mut on_event: impl FnMut(&str, &str) -> Result<()>,
) -> Result<()> {
    let mut event = String::new();
    let mut data = String::new();
    let mut has_data = false;
    read_lines(body, |line| {
        if line.is_empty() {
            if has_data {
                on_event(&event, &data)?;
            }
            event.clear();
            data.clear();
            has_data = false;
            return Ok(());
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = value.to_string(),
            "data" => {
                if has_data {
                    data.push('\n');
                }
                data.push_str(value);
                has_data = true;
            }
            _ => {}
        }
        Ok(())
    })?;
    if has_data {
        on_event(&event, &data)?;
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// Call `on_line` for each line of `body`, without its `\n` or `\r\n`.
pub(crate) fn read_lines(
    body: impl Read,
    mut on_line: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let mut reader = BufReader::new(body);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(io_error)? == 0 {
            return Ok(());
        }
        let text = line.strip_suffix('\n').unwrap_or(&line);
        on_line(text.strip_suffix('\r').unwrap_or(text))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    fn events(body: &str) -> Vec<(String, String)> {
        let mut events = Vec::new();
        read_sse(body.as_bytes(), |event, data| {
            events.push((event.to_string(), data.to_string()));
            Ok(())
        })
        .unwrap();
        events
    }

    // ------------------------------------------------------------------------
    #[test]
    fn sse_events_with_names_multiline_data_and_comments() {
        let body = ": keep-alive\r\n\r\n\
                    event: message_start\r\ndata: {\"a\":1}\r\n\r\n\
                    data: one\ndata:two\n\n\
                    event: ping\n\n\
                    data: [DONE]";
        assert_eq!(
            events(body),
            [
                ("message_start".into(), "{\"a\":1}".into()),
                (String::new(), "one\ntwo".into()),
                (String::new(), "[DONE]".into()),
            ]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn lines_without_line_endings() {
        let mut lines = Vec::new();
        read_lines("{\"a\":1}\r\n{\"b\":2}\n\nlast".as_bytes(), |line| {
            lines.push(line.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(lines, ["{\"a\":1}", "{\"b\":2}", "", "last"]);
    }
}
