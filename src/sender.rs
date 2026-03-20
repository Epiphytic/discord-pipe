use std::time::{Duration, Instant};

use crate::format::{format_code_block, format_embed};

pub struct TokenBucket {
    capacity: u32,
    tokens: u32,
    refill_period: Duration,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(capacity: u32, refill_period: Duration) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_period,
            last_refill: Instant::now(),
        }
    }

    pub fn try_acquire(&mut self) -> bool {
        self.refill();
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }

    pub fn wait_duration(&self) -> Duration {
        if self.tokens > 0 {
            return Duration::ZERO;
        }
        let elapsed = self.last_refill.elapsed();
        self.refill_period.saturating_sub(elapsed)
    }

    pub fn sync_from_headers(&mut self, remaining: u32, reset_after: f64) {
        self.tokens = remaining.min(self.capacity);
        self.last_refill = Instant::now();
        self.refill_period = Duration::from_secs_f64(reset_after);
    }

    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed();
        if elapsed >= self.refill_period {
            self.tokens = self.capacity;
            self.last_refill = Instant::now();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Code,
    Embed,
}

fn timestamp_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    format!(
        "{}-{:02}-{:02} {:02}:{:02}:{:02}",
        1970 + secs / 31_536_000,
        (secs % 31_536_000) / 2_592_000 + 1,
        (secs % 2_592_000) / 86400 + 1,
        hours,
        mins,
        s
    )
}

pub fn build_webhook_payload(
    content: &str,
    tag: &str,
    format: Format,
    username: Option<&str>,
) -> String {
    let ts = timestamp_now();
    match format {
        Format::Code => {
            let formatted = format_code_block(content, tag, &ts);
            let mut payload = serde_json::json!({ "content": formatted });
            if let Some(name) = username {
                payload["username"] = serde_json::Value::String(name.to_string());
            }
            payload.to_string()
        }
        Format::Embed => {
            let json_str = format_embed(content, tag, &ts);
            if let Some(name) = username {
                let mut payload: serde_json::Value =
                    serde_json::from_str(&json_str).unwrap_or_default();
                payload["username"] = serde_json::Value::String(name.to_string());
                payload.to_string()
            } else {
                json_str
            }
        }
    }
}

pub fn build_webhook_payload_seq(
    content: &str,
    tag: &str,
    format: Format,
    username: Option<&str>,
    seq: usize,
    total: usize,
) -> String {
    let seq_tag = format!("{tag} [{seq}/{total}]");
    build_webhook_payload(content, &seq_tag, format, username)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn build_payload_code_block() {
        let payload = build_webhook_payload("hello\nworld", "cargo build", Format::Code, None);
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(parsed["content"].as_str().unwrap().contains("```"));
        assert!(parsed["content"].as_str().unwrap().contains("hello\nworld"));
    }

    #[test]
    fn build_payload_embed() {
        let payload = build_webhook_payload("hello", "tag", Format::Embed, None);
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(parsed["embeds"].is_array());
        assert!(parsed["embeds"][0]["description"]
            .as_str()
            .unwrap()
            .contains("hello"));
    }

    #[test]
    fn build_payload_with_username() {
        let payload = build_webhook_payload("test", "tag", Format::Code, Some("MyBot"));
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["username"].as_str().unwrap(), "MyBot");
    }

    #[test]
    fn build_payload_sequence_indicator() {
        let payload = build_webhook_payload_seq("content", "tag", Format::Code, None, 2, 3);
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let content = parsed["content"].as_str().unwrap();
        assert!(content.contains("[2/3]"));
    }

    #[test]
    fn new_bucket_has_full_tokens() {
        let mut bucket = TokenBucket::new(5, Duration::from_secs(2));
        assert!(bucket.try_acquire());
    }

    #[test]
    fn bucket_drains_after_capacity() {
        let mut bucket = TokenBucket::new(5, Duration::from_secs(2));
        for _ in 0..5 {
            assert!(bucket.try_acquire());
        }
        assert!(!bucket.try_acquire());
    }

    #[test]
    fn wait_duration_returns_zero_when_available() {
        let bucket = TokenBucket::new(5, Duration::from_secs(2));
        assert_eq!(bucket.wait_duration(), Duration::ZERO);
    }

    #[test]
    fn wait_duration_returns_positive_when_empty() {
        let mut bucket = TokenBucket::new(5, Duration::from_secs(2));
        for _ in 0..5 {
            bucket.try_acquire();
        }
        assert!(bucket.wait_duration() > Duration::ZERO);
    }

    #[test]
    fn sync_from_headers_updates_remaining() {
        let mut bucket = TokenBucket::new(5, Duration::from_secs(2));
        bucket.sync_from_headers(2, 1.5);
        assert!(bucket.try_acquire());
        assert!(bucket.try_acquire());
        assert!(!bucket.try_acquire());
    }
}
