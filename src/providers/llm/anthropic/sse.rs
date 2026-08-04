use crate::{Result, RuChatError};
use serde::Deserialize;

/// One parsed Anthropic streaming event's effect on the in-progress chat response: either a
/// text delta to append, the stream-ending signal, or nothing (most event types carry no text
/// — `message_start`/`content_block_start`/`content_block_stop`/`message_delta`/`ping`).
#[derive(Debug, PartialEq)]
pub(crate) enum SseEffect {
    TextDelta(String),
    Done,
    Ignore,
}

#[derive(Deserialize)]
struct ContentBlockDeltaEvent {
    delta: Delta,
}

#[derive(Deserialize)]
struct Delta {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

/// Parses one Anthropic Messages API streaming event (an SSE frame's `event: <type>` name and
/// its `data: <json>` body, already split apart by the caller) into its effect on the chat
/// response being assembled. Anthropic's streaming shape: `message_start` → one or more
/// `content_block_start` / `content_block_delta` (the only event carrying response text,
/// incrementally) / `content_block_stop` → an optional `message_delta` (stop_reason/usage) →
/// `message_stop`, with `ping` keep-alives interspersed and a distinct `error` event for
/// mid-stream failures (e.g. `overloaded_error`, `rate_limit_error`) rather than only ever
/// failing at the initial HTTP response.
pub(crate) fn parse_event(event_type: &str, data_json: &str) -> Result<SseEffect> {
    match event_type {
        "content_block_delta" => {
            let parsed: ContentBlockDeltaEvent = serde_json::from_str(data_json).map_err(|e| {
                RuChatError::AnthropicError(format!("malformed content_block_delta event: {e}"))
            })?;
            if parsed.delta.kind == "text_delta" && !parsed.delta.text.is_empty() {
                Ok(SseEffect::TextDelta(parsed.delta.text))
            } else {
                Ok(SseEffect::Ignore)
            }
        }
        "message_stop" => Ok(SseEffect::Done),
        "error" => Err(RuChatError::AnthropicError(format!(
            "Anthropic stream error event: {data_json}"
        ))),
        _ => Ok(SseEffect::Ignore),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_block_delta_with_text_delta_yields_the_text() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        assert_eq!(
            parse_event("content_block_delta", data).unwrap(),
            SseEffect::TextDelta("Hello".to_string())
        );
    }

    #[test]
    fn message_stop_yields_done() {
        assert_eq!(
            parse_event("message_stop", r#"{"type":"message_stop"}"#).unwrap(),
            SseEffect::Done
        );
    }

    #[test]
    fn unrecognized_event_types_are_ignored() {
        for (event_type, data) in [
            ("message_start", r#"{"type":"message_start","message":{}}"#),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0}"#,
            ),
            (
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
            (
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            ),
            ("ping", r#"{"type":"ping"}"#),
        ] {
            assert_eq!(
                parse_event(event_type, data).unwrap(),
                SseEffect::Ignore,
                "expected {event_type} to be ignored"
            );
        }
    }

    // Anthropic's other documented delta kind, `input_json_delta`, only shows up for tool-use
    // content blocks — ruchat's own tool-calling convention is a text-based fenced-code-block
    // the model is prompted to produce, not the API's native tool-use feature, so this delta
    // kind should never actually occur in practice, but must not be misread as response text if
    // it ever did.
    #[test]
    fn a_non_text_delta_kind_is_ignored_not_treated_as_text() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}"#;
        assert_eq!(
            parse_event("content_block_delta", data).unwrap(),
            SseEffect::Ignore
        );
    }

    #[test]
    fn malformed_content_block_delta_json_is_a_real_error_not_a_panic() {
        let result = parse_event("content_block_delta", "not json");
        assert!(result.is_err());
    }

    #[test]
    fn a_mid_stream_error_event_surfaces_as_a_real_error() {
        let data = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        let result = parse_event("error", data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("overloaded_error"));
    }
}
