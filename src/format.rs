use serde_json::json;

const DISCORD_CONTENT_LIMIT: usize = 2000;
const DISCORD_EMBED_DESC_LIMIT: usize = 4096;

pub fn format_code_block(content: &str, tag: &str, timestamp: &str) -> String {
    let header = format!("**`[{tag}]`** {timestamp}");
    let full = format!("{header}\n```\n{content}\n```");

    if full.chars().count() > DISCORD_CONTENT_LIMIT {
        let available = DISCORD_CONTENT_LIMIT - overhead_chars(tag, "code") - timestamp.len();
        let truncated: String = content.chars().take(available.saturating_sub(4)).collect();
        format!("{header}\n```\n{truncated}...\n```")
    } else {
        full
    }
}

pub fn format_embed(content: &str, tag: &str, timestamp: &str) -> String {
    let title = format!("[{tag}] {timestamp}");
    let description = format!("```\n{content}\n```");

    let description = if description.chars().count() > DISCORD_EMBED_DESC_LIMIT {
        let available = DISCORD_EMBED_DESC_LIMIT - 8; // "```\n" + "\n```" = 8
        let truncated: String = content.chars().take(available.saturating_sub(3)).collect();
        format!("```\n{truncated}...\n```")
    } else {
        description
    };

    let payload = json!({
        "embeds": [{
            "title": title,
            "description": description,
            "color": 3066993
        }]
    });

    payload.to_string()
}

pub fn overhead_chars(tag: &str, format: &str) -> usize {
    match format {
        "code" => {
            // **`[{tag}]`** \n```\n\n```
            // Without content and timestamp, just the fixed chars + tag
            let template = format!("**`[{tag}]`** \n```\n\n```");
            template.chars().count()
        }
        "embed" => {
            // Fixed overhead in the JSON envelope, excluding content and timestamp
            let sample = format_embed("", tag, "");
            let parsed: serde_json::Value = serde_json::from_str(&sample).unwrap();
            let desc = parsed["embeds"][0]["description"].as_str().unwrap();
            // description is "```\n\n```" when content is empty = 8 chars of overhead
            // title is "[{tag}] " when timestamp is empty
            // total overhead = full json length - 0 (content) - 0 (timestamp)
            sample.len() - desc.len() + 8 // re-add the description wrapper chars
        }
        _ => 0,
    }
}

pub fn split_content(content: &str, max_chars: usize, max_messages: usize) -> Vec<String> {
    if content.chars().count() <= max_chars {
        return vec![content.to_string()];
    }

    let lines: Vec<&str> = content.split('\n').collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in &lines {
        let candidate = if current.is_empty() {
            line.to_string()
        } else {
            format!("{current}\n{line}")
        };

        if candidate.chars().count() > max_chars {
            if !current.is_empty() {
                chunks.push(current);
                current = line.to_string();
            } else {
                chunks.push(line.chars().take(max_chars).collect());
                current = String::new();
            }
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }

    if chunks.len() <= max_messages {
        return chunks;
    }

    let first = chunks.first().unwrap().clone();
    let last = chunks.last().unwrap().clone();
    let omitted_lines: usize = chunks[1..chunks.len() - 1]
        .iter()
        .map(|c| c.split('\n').count())
        .sum();
    let middle = format!("... ({omitted_lines} lines omitted)");

    vec![first, middle, last]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_code_block_with_tag_and_timestamp() {
        let msg = format_code_block("hello\nworld", "cargo build", "2026-03-20 07:15:00");
        assert!(msg.starts_with("**`[cargo build]`** 2026-03-20 07:15:00\n```\n"));
        assert!(msg.ends_with("\n```"));
        assert!(msg.contains("hello\nworld"));
    }

    #[test]
    fn formats_code_block_content_length() {
        let msg = format_code_block("test", "tag", "ts");
        assert!(msg.chars().count() <= 2000);
    }

    #[test]
    fn formats_embed_with_tag_and_content() {
        let json = format_embed("hello\nworld", "cargo build", "2026-03-20T07:15:00Z");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["embeds"][0]["title"]
            .as_str()
            .unwrap()
            .contains("cargo build"));
        assert!(parsed["embeds"][0]["description"]
            .as_str()
            .unwrap()
            .contains("hello\nworld"));
    }

    #[test]
    fn overhead_chars_code_block() {
        let tag = "cargo build";
        let overhead = overhead_chars(tag, "code");
        let msg = format_code_block("", tag, "2026-03-20 07:15:00");
        let content_len = 0_usize;
        assert_eq!(
            msg.chars().count() - content_len,
            overhead + "2026-03-20 07:15:00".len()
        );
    }

    #[test]
    fn overhead_chars_embed() {
        let tag = "deploy";
        let overhead = overhead_chars(tag, "embed");
        assert!(overhead > 0);
    }

    #[test]
    fn embed_has_correct_color() {
        let json = format_embed("test", "tag", "ts");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["embeds"][0]["color"], 3066993);
    }

    #[test]
    fn split_within_limit_returns_single_chunk() {
        let chunks = split_content("short text", 1800, 3);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "short text");
    }

    #[test]
    fn split_oversized_into_multiple_chunks() {
        let content = "a\n".repeat(100); // 200 chars across 100 lines
        let chunks = split_content(&content, 50, 3);
        assert!(chunks.len() > 1);
        assert!(chunks.len() <= 3);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 50);
        }
    }

    #[test]
    fn split_truncates_middle_when_exceeding_max_messages() {
        let content = "line\n".repeat(500); // way too long for 3 msgs
        let chunks = split_content(&content, 50, 3);
        assert_eq!(chunks.len(), 3);
        assert!(chunks[1].contains("omitted") || chunks[1].contains("truncated"));
    }

    #[test]
    fn split_preserves_line_boundaries() {
        let content = "aaaa\nbbbb\ncccc\ndddd";
        let chunks = split_content(&content, 10, 3);
        for chunk in &chunks {
            assert!(!chunk.starts_with('\n'));
        }
    }
}
