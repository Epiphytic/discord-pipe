use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: Option<String>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: Option<Vec<ContentBlock>>,
}

#[derive(Debug, Deserialize)]
struct RawEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    message: Option<Message>,
    name: Option<String>,
    input: Option<Value>,
    content: Option<Value>,
    text: Option<String>,
}

fn format_tool_args(input: &Value) -> String {
    match input {
        Value::Object(map) => {
            let pairs: Vec<String> = map
                .iter()
                .take(3)
                .map(|(k, v)| {
                    let val = match v {
                        Value::String(s) => {
                            if s.len() > 60 {
                                format!("\"{}...\"", &s[..57])
                            } else {
                                format!("\"{s}\"")
                            }
                        }
                        other => {
                            let s = other.to_string();
                            if s.len() > 60 {
                                format!("{}...", &s[..57])
                            } else {
                                s
                            }
                        }
                    };
                    format!("{k}: {val}")
                })
                .collect();
            let suffix = if map.len() > 3 { ", ..." } else { "" };
            format!("{{{}{suffix}}}", pairs.join(", "))
        }
        Value::String(s) => {
            if s.len() > 80 {
                format!("\"{}...\"", &s[..77])
            } else {
                format!("\"{s}\"")
            }
        }
        other => {
            let s = other.to_string();
            if s.len() > 80 {
                format!("{}...", &s[..77])
            } else {
                s
            }
        }
    }
}

fn extract_text_from_content_blocks(blocks: &[ContentBlock]) -> Option<String> {
    let texts: Vec<&str> = blocks
        .iter()
        .filter(|b| b.block_type.as_deref() == Some("text"))
        .filter_map(|b| b.text.as_deref())
        .filter(|t| !t.is_empty())
        .collect();

    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

fn extract_plain_content(content: &Value) -> Option<String> {
    match content {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Array(arr) => {
            let blocks: Vec<ContentBlock> = arr
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
            extract_text_from_content_blocks(&blocks)
        }
        _ => None,
    }
}

pub fn parse_ndjson_line(line: &str, show_tool_calls: bool) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let event: RawEvent = match serde_json::from_str(trimmed) {
        Ok(e) => e,
        Err(_) => return Some(trimmed.to_string()),
    };

    match event.event_type.as_deref() {
        Some("assistant") => {
            if let Some(ref msg) = event.message {
                if let Some(ref blocks) = msg.content {
                    return extract_text_from_content_blocks(blocks);
                }
            }
            None
        }

        Some("tool_use") => {
            if show_tool_calls {
                let name = event.name.unwrap_or_else(|| "unknown".to_string());
                if let Some(ref input) = event.input {
                    Some(format!("\u{1f527} {name}({})", format_tool_args(input)))
                } else {
                    Some(format!("\u{1f527} {name}()"))
                }
            } else {
                None
            }
        }

        Some("tool_result") => None,

        Some("text") => {
            let text = event
                .content
                .and_then(|c| extract_plain_content(&c))
                .or(event.text);
            text.filter(|t| !t.is_empty())
        }

        Some("tool_call") => {
            if show_tool_calls {
                let name = event.name.unwrap_or_else(|| "unknown".to_string());
                if let Some(ref input) = event.input {
                    Some(format!("\u{1f527} {name}({})", format_tool_args(input)))
                } else {
                    Some(format!("\u{1f527} {name}()"))
                }
            } else {
                None
            }
        }

        Some("token_usage") | Some("usage") | Some("stats") | Some("system") | Some("result") => {
            None
        }

        Some("error") => {
            let msg = event
                .content
                .and_then(|c| extract_plain_content(&c))
                .or(event.text)
                .unwrap_or_else(|| "unknown error".to_string());
            Some(format!("[error: {msg}]"))
        }

        Some(_) => event
            .content
            .and_then(|c| extract_plain_content(&c))
            .or(event.text),

        None => event
            .content
            .and_then(|c| extract_plain_content(&c))
            .or(event.text)
            .or_else(|| Some(trimmed.to_string())),
    }
}

pub struct NdjsonFilter<I> {
    inner: I,
    show_tool_calls: bool,
}

impl<I> NdjsonFilter<I> {
    pub fn new(inner: I, show_tool_calls: bool) -> Self {
        Self {
            inner,
            show_tool_calls,
        }
    }
}

impl<I: Iterator<Item = String>> Iterator for NdjsonFilter<I> {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        loop {
            let line = self.inner.next()?;
            if let Some(text) = parse_ndjson_line(&line, self.show_tool_calls) {
                return Some(text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_from_assistant_message() {
        let line =
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello world"}]}}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("Hello world".to_string())
        );
    }

    #[test]
    fn extracts_multiple_text_blocks_from_assistant() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"First"},{"type":"text","text":"Second"}]}}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("First\nSecond".to_string())
        );
    }

    #[test]
    fn filters_tool_use_by_default() {
        let line = r#"{"type":"tool_use","name":"file_read","input":{"file_path":"src/main.rs"}}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn shows_tool_use_with_wrench_emoji_when_enabled() {
        let line = r#"{"type":"tool_use","name":"file_read","input":{"file_path":"src/main.rs"}}"#;
        let result = parse_ndjson_line(line, true).unwrap();
        assert!(result.starts_with("\u{1f527} file_read("));
        assert!(result.contains("file_path"));
        assert!(result.contains("src/main.rs"));
    }

    #[test]
    fn tool_use_without_input_shows_empty_parens() {
        let line = r#"{"type":"tool_use","name":"list"}"#;
        assert_eq!(
            parse_ndjson_line(line, true),
            Some("\u{1f527} list()".to_string())
        );
    }

    #[test]
    fn filters_tool_result() {
        let line = r#"{"type":"tool_result","content":"some output here"}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
        assert_eq!(parse_ndjson_line(line, true), None);
    }

    #[test]
    fn filters_token_usage_events() {
        let line = r#"{"type":"token_usage","input":100,"output":50}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
        assert_eq!(parse_ndjson_line(line, true), None);
    }

    #[test]
    fn filters_usage_events() {
        let line = r#"{"type":"usage","total_tokens":150}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn filters_stats_events() {
        let line = r#"{"type":"stats","duration_ms":1234}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn filters_system_events() {
        let line = r#"{"type":"system","content":"init"}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn filters_result_events() {
        let line = r#"{"type":"result","content":"done"}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn passes_error_events_through() {
        let line = r#"{"type":"error","content":"something broke"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("[error: something broke]".to_string())
        );
    }

    #[test]
    fn error_without_content_shows_unknown() {
        let line = r#"{"type":"error"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("[error: unknown error]".to_string())
        );
    }

    #[test]
    fn non_json_lines_pass_through_as_plain_text() {
        let line = "This is just plain text output";
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("This is just plain text output".to_string())
        );
    }

    #[test]
    fn invalid_json_passes_through_as_plain_text() {
        let line = "{not valid json at all";
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("{not valid json at all".to_string())
        );
    }

    #[test]
    fn empty_lines_are_filtered() {
        assert_eq!(parse_ndjson_line("", false), None);
        assert_eq!(parse_ndjson_line("  ", false), None);
        assert_eq!(parse_ndjson_line("\t", false), None);
    }

    #[test]
    fn assistant_with_empty_text_is_filtered() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":""}]}}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn assistant_without_message_is_filtered() {
        let line = r#"{"type":"assistant"}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn backward_compat_text_event_with_content_string() {
        let line = r#"{"type":"text","content":"Hello world"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("Hello world".to_string())
        );
    }

    #[test]
    fn backward_compat_text_event_with_text_field() {
        let line = r#"{"type":"text","text":"Hello from text field"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("Hello from text field".to_string())
        );
    }

    #[test]
    fn backward_compat_tool_call_event() {
        let line = r#"{"type":"tool_call","name":"read","content":"reading file"}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn backward_compat_tool_call_shown_when_enabled() {
        let line = r#"{"type":"tool_call","name":"read"}"#;
        let result = parse_ndjson_line(line, true).unwrap();
        assert!(result.starts_with("\u{1f527} read("));
    }

    #[test]
    fn tool_args_truncated_for_long_strings() {
        let long_path = "a".repeat(100);
        let line =
            format!(r#"{{"type":"tool_use","name":"read","input":{{"file_path":"{long_path}"}}}}"#);
        let result = parse_ndjson_line(&line, true).unwrap();
        assert!(result.contains("..."));
        assert!(result.len() < 200);
    }

    #[test]
    fn tool_args_limits_to_three_keys() {
        let line = r#"{"type":"tool_use","name":"edit","input":{"file_path":"a.rs","old_string":"x","new_string":"y","extra":"z"}}"#;
        let result = parse_ndjson_line(&line, true).unwrap();
        assert!(result.contains("..."));
    }

    #[test]
    fn ndjson_filter_iterator_skips_noise() {
        let lines = vec![
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}"#
                .to_string(),
            r#"{"type":"tool_use","name":"read","input":{"file_path":"x"}}"#.to_string(),
            r#"{"type":"tool_result","content":"file content"}"#.to_string(),
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}"#
                .to_string(),
            r#"{"type":"token_usage","input":100}"#.to_string(),
        ];

        let filter = NdjsonFilter::new(lines.into_iter(), false);
        let result: Vec<String> = filter.collect();
        assert_eq!(result, vec!["Hello", "Done"]);
    }

    #[test]
    fn ndjson_filter_with_tool_calls_enabled() {
        let lines = vec![
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Checking"}]}}"#
                .to_string(),
            r#"{"type":"tool_use","name":"grep","input":{"pattern":"TODO"}}"#.to_string(),
            r#"{"type":"tool_result","content":"found 3"}"#.to_string(),
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Found items"}]}}"#
                .to_string(),
        ];

        let filter = NdjsonFilter::new(lines.into_iter(), true);
        let result: Vec<String> = filter.collect();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Checking");
        assert!(result[1].contains("grep"));
        assert!(result[1].contains("TODO"));
        assert_eq!(result[2], "Found items");
    }

    #[test]
    fn unknown_json_type_with_content_passes_through() {
        let line = r#"{"type":"custom","content":"hello"}"#;
        assert_eq!(parse_ndjson_line(line, false), Some("hello".to_string()));
    }

    #[test]
    fn no_type_field_with_text_passes_through() {
        let line = r#"{"text":"fallback text"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("fallback text".to_string())
        );
    }

    #[test]
    fn no_type_no_content_passes_raw_line() {
        let line = r#"{"foo":"bar"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some(r#"{"foo":"bar"}"#.to_string())
        );
    }
}
