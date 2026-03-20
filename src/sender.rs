use std::thread;
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

pub struct HttpResponse {
    pub status: u16,
    pub rate_limit_remaining: Option<u32>,
    pub rate_limit_reset_after: Option<f64>,
    pub retry_after: Option<f64>,
}

#[derive(Debug)]
pub enum SendError {
    RateLimited,
    Permanent(u16),
    Transient(String),
    Network(String),
}

pub trait HttpPoster {
    fn post(&self, url: &str, body: &str) -> Result<HttpResponse, SendError>;
}

pub struct UreqPoster;

impl HttpPoster for UreqPoster {
    fn post(&self, url: &str, body: &str) -> Result<HttpResponse, SendError> {
        let response = ureq::post(url)
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|e: ureq::Error| SendError::Network(e.to_string()))?;

        let status = response.status().as_u16();

        let header_str = |name: &str| -> Option<String> {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        };

        let rate_limit_remaining = header_str("X-RateLimit-Remaining").and_then(|v| v.parse().ok());

        let rate_limit_reset_after =
            header_str("X-RateLimit-Reset-After").and_then(|v| v.parse().ok());

        let retry_after = header_str("Retry-After").and_then(|v| v.parse().ok());

        Ok(HttpResponse {
            status,
            rate_limit_remaining,
            rate_limit_reset_after,
            retry_after,
        })
    }
}

pub struct Sender<P: HttpPoster> {
    pub poster: P,
    url: String,
    bucket: TokenBucket,
}

const MAX_RETRIES: usize = 3;

impl<P: HttpPoster> Sender<P> {
    pub fn new(poster: P, url: &str, bucket: TokenBucket) -> Self {
        Self {
            poster,
            url: url.to_string(),
            bucket,
        }
    }

    pub fn send_batch(
        &mut self,
        content: &str,
        tag: &str,
        format: Format,
        username: Option<&str>,
    ) -> Result<(), SendError> {
        let payload = build_webhook_payload(content, tag, format, username);

        let wait = self.bucket.wait_duration();
        if !wait.is_zero() {
            thread::sleep(wait);
        }
        self.bucket.try_acquire();

        let mut retries = 0;
        loop {
            let resp = self.poster.post(&self.url, &payload)?;

            if let (Some(remaining), Some(reset_after)) =
                (resp.rate_limit_remaining, resp.rate_limit_reset_after)
            {
                self.bucket.sync_from_headers(remaining, reset_after);
            }

            match resp.status {
                200..=299 => return Ok(()),
                429 => {
                    let wait = resp.retry_after.unwrap_or(1.0);
                    thread::sleep(Duration::from_secs_f64(wait));
                    retries += 1;
                    if retries >= MAX_RETRIES {
                        return Err(SendError::RateLimited);
                    }
                }
                s @ 500..=599 => {
                    retries += 1;
                    if retries >= MAX_RETRIES {
                        return Err(SendError::Transient(format!("server error: {s}")));
                    }
                    let backoff = Duration::from_secs(1 << (retries - 1));
                    thread::sleep(backoff);
                }
                s @ 400..=499 => {
                    return Err(SendError::Permanent(s));
                }
                s => {
                    return Err(SendError::Transient(format!("unexpected status: {s}")));
                }
            }
        }
    }
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

    struct MockPoster {
        responses: std::cell::RefCell<Vec<HttpResponse>>,
        call_count: std::cell::Cell<usize>,
    }

    impl MockPoster {
        fn new(responses: Vec<HttpResponse>) -> Self {
            Self {
                responses: std::cell::RefCell::new(responses),
                call_count: std::cell::Cell::new(0),
            }
        }
        fn call_count(&self) -> usize {
            self.call_count.get()
        }
    }

    impl HttpPoster for MockPoster {
        fn post(&self, _url: &str, _body: &str) -> Result<HttpResponse, SendError> {
            self.call_count.set(self.call_count.get() + 1);
            if self.responses.borrow().is_empty() {
                return Err(SendError::Network("no more responses".into()));
            }
            Ok(self.responses.borrow_mut().remove(0))
        }
    }

    fn test_bucket() -> TokenBucket {
        TokenBucket::new(5, Duration::from_millis(100))
    }

    #[test]
    fn sender_posts_successfully() {
        let mock = MockPoster::new(vec![HttpResponse {
            status: 204,
            rate_limit_remaining: None,
            rate_limit_reset_after: None,
            retry_after: None,
        }]);
        let mut sender = Sender::new(mock, "https://webhook.url", test_bucket());
        let result = sender.send_batch("content", "tag", Format::Code, None);
        assert!(result.is_ok());
        assert_eq!(sender.poster.call_count(), 1);
    }

    #[test]
    fn sender_retries_on_429() {
        let mock = MockPoster::new(vec![
            HttpResponse {
                status: 429,
                retry_after: Some(0.01),
                rate_limit_remaining: None,
                rate_limit_reset_after: None,
            },
            HttpResponse {
                status: 204,
                rate_limit_remaining: None,
                rate_limit_reset_after: None,
                retry_after: None,
            },
        ]);
        let mut sender = Sender::new(mock, "https://webhook.url", test_bucket());
        let result = sender.send_batch("content", "tag", Format::Code, None);
        assert!(result.is_ok());
        assert_eq!(sender.poster.call_count(), 2);
    }

    #[test]
    fn sender_retries_on_5xx_up_to_3_times() {
        let mock = MockPoster::new(vec![
            HttpResponse {
                status: 500,
                rate_limit_remaining: None,
                rate_limit_reset_after: None,
                retry_after: None,
            },
            HttpResponse {
                status: 502,
                rate_limit_remaining: None,
                rate_limit_reset_after: None,
                retry_after: None,
            },
            HttpResponse {
                status: 503,
                rate_limit_remaining: None,
                rate_limit_reset_after: None,
                retry_after: None,
            },
        ]);
        let mut sender = Sender::new(mock, "https://webhook.url", test_bucket());
        let result = sender.send_batch("content", "tag", Format::Code, None);
        assert!(result.is_err());
        assert_eq!(sender.poster.call_count(), 3);
    }

    #[test]
    fn sender_does_not_retry_on_401() {
        let mock = MockPoster::new(vec![HttpResponse {
            status: 401,
            rate_limit_remaining: None,
            rate_limit_reset_after: None,
            retry_after: None,
        }]);
        let mut sender = Sender::new(mock, "https://webhook.url", test_bucket());
        let result = sender.send_batch("content", "tag", Format::Code, None);
        assert!(result.is_err());
        assert_eq!(sender.poster.call_count(), 1);
    }

    #[test]
    fn sender_syncs_rate_limit_from_headers() {
        let mock = MockPoster::new(vec![HttpResponse {
            status: 204,
            rate_limit_remaining: Some(1),
            rate_limit_reset_after: Some(1.5),
            retry_after: None,
        }]);
        let mut sender = Sender::new(mock, "https://webhook.url", test_bucket());
        sender
            .send_batch("content", "tag", Format::Code, None)
            .unwrap();
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
