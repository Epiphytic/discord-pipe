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
}
