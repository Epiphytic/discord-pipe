use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RawEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    content: Option<String>,
    text: Option<String>,
    name: Option<String>,
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
        Some("text") => {
            let text = event.content.or(event.text).unwrap_or_default();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        Some("tool_call") | Some("tool_result") => {
            if show_tool_calls {
                let name = event.name.unwrap_or_else(|| "unknown".to_string());
                Some(format!("[tool: {name}]"))
            } else {
                None
            }
        }
        Some("token_usage") | Some("usage") | Some("stats") => None,
        Some("error") => {
            let msg = event
                .content
                .or(event.text)
                .unwrap_or_else(|| "unknown error".to_string());
            Some(format!("[error: {msg}]"))
        }
        Some(_) => event.content.or(event.text),
        None => event
            .content
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
    fn extracts_text_from_content_field() {
        let line = r#"{"type":"text","content":"Hello world"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("Hello world".to_string())
        );
    }

    #[test]
    fn extracts_text_from_text_field() {
        let line = r#"{"type":"text","text":"Hello from text field"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("Hello from text field".to_string())
        );
    }

    #[test]
    fn prefers_content_over_text_field() {
        let line = r#"{"type":"text","content":"from content","text":"from text"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("from content".to_string())
        );
    }

    #[test]
    fn filters_tool_call_by_default() {
        let line = r#"{"type":"tool_call","name":"read","content":"reading file"}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn shows_tool_call_when_enabled() {
        let line = r#"{"type":"tool_call","name":"read","content":"reading file"}"#;
        assert_eq!(
            parse_ndjson_line(line, true),
            Some("[tool: read]".to_string())
        );
    }

    #[test]
    fn filters_tool_result_by_default() {
        let line = r#"{"type":"tool_result","name":"grep","content":"found 3 matches"}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn shows_tool_result_when_enabled() {
        let line = r#"{"type":"tool_result","name":"grep"}"#;
        assert_eq!(
            parse_ndjson_line(line, true),
            Some("[tool: grep]".to_string())
        );
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
    fn text_event_with_empty_content_is_filtered() {
        let line = r#"{"type":"text","content":""}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn json_without_type_extracts_content() {
        let line = r#"{"content":"bare content"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("bare content".to_string())
        );
    }

    #[test]
    fn json_without_type_or_content_passes_raw() {
        let line = r#"{"level":"info","msg":"server started"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some(r#"{"level":"info","msg":"server started"}"#.to_string())
        );
    }

    #[test]
    fn unknown_event_type_with_content_passes_content() {
        let line = r#"{"type":"debug","content":"debug info here"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("debug info here".to_string())
        );
    }

    #[test]
    fn unknown_event_type_without_content_returns_none() {
        let line = r#"{"type":"heartbeat"}"#;
        assert_eq!(parse_ndjson_line(line, false), None);
    }

    #[test]
    fn multiline_content_preserved() {
        let line = r#"{"type":"text","content":"line1\nline2\nline3"}"#;
        let result = parse_ndjson_line(line, false).unwrap();
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
    }

    #[test]
    fn unicode_content_preserved() {
        let line = r#"{"type":"text","content":"Hello 🌍 café"}"#;
        assert_eq!(
            parse_ndjson_line(line, false),
            Some("Hello 🌍 café".to_string())
        );
    }

    #[test]
    fn filter_iterator_extracts_text_events() {
        let lines = vec![
            r#"{"type":"text","content":"hello"}"#.to_string(),
            r#"{"type":"tool_call","name":"read"}"#.to_string(),
            r#"{"type":"text","content":"world"}"#.to_string(),
            r#"{"type":"token_usage","input":100}"#.to_string(),
        ];
        let filter = NdjsonFilter::new(lines.into_iter(), false);
        let result: Vec<String> = filter.collect();
        assert_eq!(result, vec!["hello", "world"]);
    }

    #[test]
    fn filter_iterator_with_tool_calls() {
        let lines = vec![
            r#"{"type":"text","content":"hello"}"#.to_string(),
            r#"{"type":"tool_call","name":"grep"}"#.to_string(),
            r#"{"type":"text","content":"world"}"#.to_string(),
        ];
        let filter = NdjsonFilter::new(lines.into_iter(), true);
        let result: Vec<String> = filter.collect();
        assert_eq!(result, vec!["hello", "[tool: grep]", "world"]);
    }

    #[test]
    fn filter_handles_mixed_json_and_plain_text() {
        let lines = vec![
            "plain text line".to_string(),
            r#"{"type":"text","content":"json text"}"#.to_string(),
            "another plain line".to_string(),
        ];
        let filter = NdjsonFilter::new(lines.into_iter(), false);
        let result: Vec<String> = filter.collect();
        assert_eq!(
            result,
            vec!["plain text line", "json text", "another plain line"]
        );
    }

    #[test]
    fn filter_skips_empty_lines() {
        let lines = vec![
            "".to_string(),
            r#"{"type":"text","content":"hello"}"#.to_string(),
            "  ".to_string(),
        ];
        let filter = NdjsonFilter::new(lines.into_iter(), false);
        let result: Vec<String> = filter.collect();
        assert_eq!(result, vec!["hello"]);
    }
}
