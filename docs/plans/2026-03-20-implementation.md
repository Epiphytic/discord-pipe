# Plan: discord-pipe Full Implementation

**Date:** 2026-03-20
**Status:** In Progress

## Progress Log

- **15:31 UTC** - Phase 1 complete: Cargo project skeleton with all modules, clippy clean
- **15:32 UTC** - Phase 2 complete: ANSI stripping (7 tests)
- **15:35 UTC** - Phase 3 complete: Code block + embed formatting (6 tests)
- **15:36 UTC** - Phase 4a complete: Batch buffer (6 tests)
- **15:38 UTC** - Phase 4b complete: Overflow splitting (4 tests)
- **15:40 UTC** - Phase 5 complete: Token bucket rate limiter (5 tests)
- **15:47 UTC** - Phase 6 complete: Webhook payload + HTTP sender with retry (14 tests total)
- **15:51 UTC** - Phase 7 complete: Stdin reader + file tail (8 tests)
- **15:55 UTC** - Phase 8 complete: CLI argument parsing (5 tests)
- **Total tests: 50 passing, clippy clean**
- **Next:** Phase 9 (main loop integration), then Phase 10-11

## Context

This plan implements the `discord-pipe` CLI tool based on the approved design decisions from
the [brainstorm](../brainstorm-2026-03-20.md). Key deviations from the brainstorm's
recommendations (decided during review):

- **No WASM at all** - not deferred, removed entirely. No WASM feature flags, no wasm32 targets.
- **Both stdin AND file tail from the start** - `--follow <path>` is not deferred.
- **ANSI escape codes** - strip by default, `--no-strip-ansi` to keep them.

## Scope

- Rust binary (`discord-pipe`) that reads from stdin or tails a file
- Hybrid batching: time window (2s) + line count (50) + byte/char size (1800 Unicode codepoints)
- Discord webhook posting via `ureq` (blocking, two-thread model)
- Token bucket rate limiting + `X-RateLimit` header sync + 429 retry with backoff
- Overflow: split to 3 messages max, truncate middle if still oversized
- Code block formatting by default, `--format embed` as option
- ANSI stripping by default
- Bounded retry (3x exponential backoff) + bounded queue (50 batches) + drop with report
- CLI via `clap` derive, env var support for webhook URL

## Out of Scope

- WASM compilation (explicitly removed)
- Named pipe input
- Multiple webhook targets
- Message editing/threading
- Discord bot token auth
- Disk-spill queue

## Approach

Two-thread producer/consumer architecture:

1. **Main thread (Reader + Batcher):** Reads lines from stdin or tails a file. Accumulates
   lines into a batch buffer. Flushes to a bounded channel when any trigger fires (time window
   expires, line count reached, byte size reached, EOF).

2. **Sender thread:** Pops `Batch` messages from the channel. Applies token bucket rate
   limiting. Formats the batch (code block or embed). Splits oversized messages. Posts via
   `ureq`. Parses rate limit headers to sync the token bucket. Handles retries with
   exponential backoff.

Communication between threads: `std::sync::mpsc` bounded-like channel (or `crossbeam-channel`).

Each implementation step follows TDD: write a failing test first, confirm it fails, implement
the code, confirm the test passes, then commit.

## Implementation Steps

Each step follows the cycle: **write failing test -> confirm fail -> implement -> confirm pass -> commit**.

---

### Phase 1: Project Skeleton

#### Step 1: Initialize Cargo project
- [ ] `cargo init --name discord-pipe`
- [ ] Add initial dependencies to `Cargo.toml`:
  - `clap` (derive feature)
  - `ureq`
  - `serde` + `serde_json`
  - `ctrlc`
  - `dotenvy`
- [ ] Add dev-dependencies: (none initially beyond built-in `#[test]`)
- [ ] Create module structure:
  - `src/main.rs` - entry point, CLI parsing
  - `src/batcher.rs` - batching logic
  - `src/sender.rs` - HTTP posting + rate limiting
  - `src/format.rs` - message formatting
  - `src/reader.rs` - input source abstraction (stdin + file tail)
  - `src/ansi.rs` - ANSI escape code stripping
- [ ] `cargo clippy --workspace -- -D warnings` clean
- [ ] Commit: `chore: initialize cargo project with module skeleton`

---

### Phase 2: ANSI Stripping

#### Step 2: ANSI escape code stripping

**Test first** (`src/ansi.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_simple_color_code() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strips_bold_and_reset() {
        assert_eq!(strip_ansi("\x1b[1mbold\x1b[0m"), "bold");
    }

    #[test]
    fn preserves_plain_text() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn strips_256_color() {
        assert_eq!(strip_ansi("\x1b[38;5;196mred\x1b[0m"), "red");
    }

    #[test]
    fn strips_rgb_color() {
        assert_eq!(strip_ansi("\x1b[38;2;255;0;0mred\x1b[0m"), "red");
    }

    #[test]
    fn strips_cursor_movement() {
        assert_eq!(strip_ansi("\x1b[2J\x1b[Hstart"), "start");
    }

    #[test]
    fn handles_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }
}
```

- [ ] Write tests -> `cargo test` -> confirm fail (function doesn't exist)
- [ ] Implement `strip_ansi(input: &str) -> String` using a state machine or regex
      to match `\x1b\[[0-9;]*[A-Za-z]` and strip it. No external crate needed.
- [ ] `cargo test` -> confirm pass
- [ ] `cargo clippy --workspace -- -D warnings` clean
- [ ] Commit: `feat: add ANSI escape code stripping`

---

### Phase 3: Message Formatting

#### Step 3: Code block formatter

**Test first** (`src/format.rs`):
```rust
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
        // Verify total length is within 2000 chars
        assert!(msg.chars().count() <= 2000);
    }

    #[test]
    fn formats_embed_with_tag_and_content() {
        let json = format_embed("hello\nworld", "cargo build", "2026-03-20T07:15:00Z");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["embeds"][0]["title"].as_str().unwrap().contains("cargo build"));
        assert!(parsed["embeds"][0]["description"].as_str().unwrap().contains("hello\nworld"));
    }
}
```

- [ ] Write tests -> `cargo test` -> confirm fail
- [ ] Implement `format_code_block(content, tag, timestamp) -> String`
- [ ] Implement `format_embed(content, tag, timestamp) -> String`
- [ ] Implement `overhead_chars(tag, format) -> usize` to calculate formatting overhead
- [ ] `cargo test` -> confirm pass
- [ ] Commit: `feat: add message formatting (code block + embed)`

---

### Phase 4: Batching Logic

#### Step 4: Batch buffer - line accumulation and byte counting

**Test first** (`src/batcher.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_batch_is_empty() {
        let batch = BatchBuffer::new(50, 1800);
        assert!(batch.is_empty());
        assert_eq!(batch.line_count(), 0);
        assert_eq!(batch.char_count(), 0);
    }

    #[test]
    fn push_line_updates_counts() {
        let mut batch = BatchBuffer::new(50, 1800);
        batch.push_line("hello");
        assert_eq!(batch.line_count(), 1);
        assert_eq!(batch.char_count(), 5);
        assert!(!batch.is_empty());
    }

    #[test]
    fn triggers_on_line_count() {
        let mut batch = BatchBuffer::new(3, 1800);
        batch.push_line("a");
        assert!(!batch.should_flush());
        batch.push_line("b");
        assert!(!batch.should_flush());
        batch.push_line("c");
        assert!(batch.should_flush());
    }

    #[test]
    fn triggers_on_char_count() {
        let mut batch = BatchBuffer::new(50, 10);
        batch.push_line("12345678901"); // 11 chars > 10
        assert!(batch.should_flush());
    }

    #[test]
    fn drain_returns_content_and_resets() {
        let mut batch = BatchBuffer::new(50, 1800);
        batch.push_line("hello");
        batch.push_line("world");
        let content = batch.drain();
        assert_eq!(content, "hello\nworld");
        assert!(batch.is_empty());
        assert_eq!(batch.line_count(), 0);
    }

    #[test]
    fn counts_unicode_codepoints_not_bytes() {
        let mut batch = BatchBuffer::new(50, 10);
        batch.push_line("héllo"); // 5 codepoints, but 6 bytes
        assert_eq!(batch.char_count(), 5);
    }
}
```

- [ ] Write tests -> `cargo test` -> confirm fail
- [ ] Implement `BatchBuffer` struct with:
  - `new(max_lines, max_chars) -> Self`
  - `push_line(&mut self, line: &str)`
  - `should_flush(&self) -> bool`
  - `drain(&mut self) -> String`
  - `is_empty(&self) -> bool`
  - `line_count(&self) -> usize`
  - `char_count(&self) -> usize`
- [ ] Char count uses `.chars().count()` (Unicode codepoints)
- [ ] `cargo test` -> confirm pass
- [ ] Commit: `feat: add batch buffer with line/char thresholds`

#### Step 5: Overflow splitting

**Test first** (`src/format.rs` or `src/batcher.rs`):
```rust
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
    // Middle chunk should contain truncation indicator
    assert!(chunks[1].contains("omitted") || chunks[1].contains("truncated"));
}

#[test]
fn split_preserves_line_boundaries() {
    let content = "aaaa\nbbbb\ncccc\ndddd";
    let chunks = split_content(&content, 10, 3);
    // Should not split in the middle of a line
    for chunk in &chunks {
        assert!(!chunk.starts_with('\n'));
    }
}
```

- [ ] Write tests -> `cargo test` -> confirm fail
- [ ] Implement `split_content(content: &str, max_chars: usize, max_messages: usize) -> Vec<String>`
  - Split on line boundaries
  - If content fits in one chunk, return single chunk
  - If content fits in `max_messages` chunks, split evenly on line boundaries
  - If content exceeds `max_messages` chunks, keep first chunk + last chunk, replace
    middle with truncation indicator (`... (N lines omitted)`)
- [ ] `cargo test` -> confirm pass
- [ ] Commit: `feat: add overflow splitting with middle truncation`

---

### Phase 5: Rate Limiting

#### Step 6: Token bucket rate limiter

**Test first** (`src/sender.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn new_bucket_has_full_tokens() {
        let bucket = TokenBucket::new(5, Duration::from_secs(2));
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
        // Simulate Discord saying 2 remaining, reset in 1.5s
        bucket.sync_from_headers(2, 1.5);
        // Should have 2 tokens available
        assert!(bucket.try_acquire());
        assert!(bucket.try_acquire());
        assert!(!bucket.try_acquire());
    }
}
```

- [ ] Write tests -> `cargo test` -> confirm fail
- [ ] Implement `TokenBucket` struct:
  - `new(capacity: u32, refill_period: Duration) -> Self`
  - `try_acquire(&mut self) -> bool`
  - `wait_duration(&self) -> Duration`
  - `sync_from_headers(&mut self, remaining: u32, reset_after: f64)`
- [ ] `cargo test` -> confirm pass
- [ ] Commit: `feat: add token bucket rate limiter with header sync`

---

### Phase 6: Discord Webhook Client

#### Step 7: Webhook payload construction + dry-run mode

**Test first** (`src/sender.rs`):
```rust
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
    assert!(parsed["embeds"][0]["description"].as_str().unwrap().contains("hello"));
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
```

- [ ] Write tests -> `cargo test` -> confirm fail
- [ ] Implement payload construction functions
- [ ] `cargo test` -> confirm pass
- [ ] Commit: `feat: add webhook payload construction`

#### Step 8: HTTP sender with retry logic (unit-testable with trait)

**Test first** (`src/sender.rs`):
```rust
// Trait for HTTP posting - allows mock in tests
trait HttpPoster {
    fn post(&self, url: &str, body: &str) -> Result<HttpResponse, SendError>;
}

struct HttpResponse {
    status: u16,
    rate_limit_remaining: Option<u32>,
    rate_limit_reset_after: Option<f64>,
    retry_after: Option<f64>,
}

#[test]
fn sender_posts_successfully() {
    let mock = MockPoster::new(vec![HttpResponse { status: 204, ..default() }]);
    let mut sender = Sender::new(mock, "https://webhook.url", token_bucket());
    let result = sender.send_batch("content", "tag", Format::Code);
    assert!(result.is_ok());
    assert_eq!(mock.call_count(), 1);
}

#[test]
fn sender_retries_on_429() {
    let mock = MockPoster::new(vec![
        HttpResponse { status: 429, retry_after: Some(0.1), ..default() },
        HttpResponse { status: 204, ..default() },
    ]);
    let mut sender = Sender::new(mock, "https://webhook.url", token_bucket());
    let result = sender.send_batch("content", "tag", Format::Code);
    assert!(result.is_ok());
    assert_eq!(mock.call_count(), 2);
}

#[test]
fn sender_retries_on_5xx_up_to_3_times() {
    let mock = MockPoster::new(vec![
        HttpResponse { status: 500, ..default() },
        HttpResponse { status: 502, ..default() },
        HttpResponse { status: 503, ..default() },
    ]);
    let mut sender = Sender::new(mock, "https://webhook.url", token_bucket());
    let result = sender.send_batch("content", "tag", Format::Code);
    assert!(result.is_err());
    assert_eq!(mock.call_count(), 3);
}

#[test]
fn sender_does_not_retry_on_401() {
    let mock = MockPoster::new(vec![
        HttpResponse { status: 401, ..default() },
    ]);
    let mut sender = Sender::new(mock, "https://webhook.url", token_bucket());
    let result = sender.send_batch("content", "tag", Format::Code);
    assert!(result.is_err());
    assert_eq!(mock.call_count(), 1);
}

#[test]
fn sender_syncs_rate_limit_from_headers() {
    let mock = MockPoster::new(vec![
        HttpResponse { status: 204, rate_limit_remaining: Some(1), rate_limit_reset_after: Some(1.5), ..default() },
    ]);
    let mut sender = Sender::new(mock, "https://webhook.url", token_bucket());
    sender.send_batch("content", "tag", Format::Code).unwrap();
    // After syncing, bucket should reflect the Discord headers
    // (we can verify through wait_duration or available tokens)
}
```

- [ ] Write tests -> `cargo test` -> confirm fail
- [ ] Implement `HttpPoster` trait + `UreqPoster` (real impl) + `MockPoster` (test impl)
- [ ] Implement `Sender` struct:
  - `new(poster: impl HttpPoster, url: &str, bucket: TokenBucket) -> Self`
  - `send_batch(&mut self, content: &str, tag: &str, format: Format) -> Result<(), SendError>`
  - Internal: exponential backoff (1s, 2s, 4s), max 3 retries
  - Internal: sync token bucket from response headers
  - Internal: handle overflow splitting (call `split_content`, send multiple)
- [ ] Implement `SendError` enum: `RateLimited`, `Permanent(u16)`, `Transient(String)`, `Network(String)`
- [ ] `cargo test` -> confirm pass
- [ ] Commit: `feat: add HTTP sender with retry and rate limit sync`

---

### Phase 7: Input Readers

#### Step 9: Stdin reader

**Test first** (`src/reader.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_lines_from_buffered_input() {
        let input = Cursor::new("line1\nline2\nline3\n");
        let reader = LineReader::new(input);
        let lines: Vec<String> = reader.collect();
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn handles_empty_input() {
        let input = Cursor::new("");
        let reader = LineReader::new(input);
        let lines: Vec<String> = reader.collect();
        assert!(lines.is_empty());
    }

    #[test]
    fn handles_line_without_trailing_newline() {
        let input = Cursor::new("no newline");
        let reader = LineReader::new(input);
        let lines: Vec<String> = reader.collect();
        assert_eq!(lines, vec!["no newline"]);
    }

    #[test]
    fn strips_ansi_when_enabled() {
        let input = Cursor::new("\x1b[31mred\x1b[0m\n");
        let reader = LineReader::with_ansi_strip(input, true);
        let lines: Vec<String> = reader.collect();
        assert_eq!(lines, vec!["red"]);
    }

    #[test]
    fn preserves_ansi_when_disabled() {
        let input = Cursor::new("\x1b[31mred\x1b[0m\n");
        let reader = LineReader::with_ansi_strip(input, false);
        let lines: Vec<String> = reader.collect();
        assert_eq!(lines, vec!["\x1b[31mred\x1b[0m"]);
    }
}
```

- [ ] Write tests -> `cargo test` -> confirm fail
- [ ] Implement `LineReader<R: BufRead>`:
  - `new(reader: R) -> Self`
  - `with_ansi_strip(reader: R, strip: bool) -> Self`
  - Implements `Iterator<Item = String>`
  - Optionally calls `strip_ansi()` on each line
- [ ] `cargo test` -> confirm pass
- [ ] Commit: `feat: add stdin line reader with optional ANSI stripping`

#### Step 10: File tail reader

**Test first** (`src/reader.rs`):
```rust
#[test]
fn tail_reads_new_lines_appended_to_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.log");
    std::fs::write(&path, "existing\n").unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        tail_file(&path, tx, true, false); // strip_ansi=true, from_start=false
    });

    // Append a line
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(f, "new line").unwrap();

    // Should receive the new line within a reasonable time
    let line = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(line, "new line");
}

#[test]
fn tail_starts_from_end_of_file_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.log");
    std::fs::write(&path, "old line\n").unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let _handle = std::thread::spawn(move || {
        tail_file(&path, tx, true, false);
    });

    // Should NOT receive "old line"
    let result = rx.recv_timeout(Duration::from_millis(500));
    assert!(result.is_err()); // timeout, no lines received
}

#[test]
fn tail_handles_file_not_existing_yet() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.log");

    let (tx, rx) = std::sync::mpsc::channel();
    let path_clone = path.clone();
    let _handle = std::thread::spawn(move || {
        tail_file(&path_clone, tx, true, false);
    });

    // Create the file after a delay
    std::thread::sleep(Duration::from_millis(200));
    use std::io::Write;
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "appeared").unwrap();

    let line = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(line, "appeared");
}
```

- [ ] Add `notify` crate to `Cargo.toml` (for filesystem events) + `tempfile` to dev-deps
- [ ] Write tests -> `cargo test` -> confirm fail
- [ ] Implement `tail_file(path, sender, strip_ansi, from_start)`:
  - Seek to end of file (unless `from_start`)
  - Use `notify` for filesystem events, polling fallback
  - Read new bytes on each event, split into lines
  - Send lines through `mpsc::Sender<String>`
  - Handle: file doesn't exist (poll until it does), file truncated (re-seek to start)
- [ ] `cargo test` -> confirm pass
- [ ] Commit: `feat: add file tail reader with notify`

---

### Phase 8: CLI Parsing

#### Step 11: CLI argument parsing with clap

**Test first** (`src/main.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_requires_webhook_url() {
        let result = Cli::try_parse_from(["discord-pipe"]);
        assert!(result.is_err()); // webhook is required (unless env var)
    }

    #[test]
    fn cli_parses_webhook_flag() {
        let cli = Cli::try_parse_from([
            "discord-pipe", "--webhook", "https://discord.com/api/webhooks/123/abc"
        ]).unwrap();
        assert_eq!(cli.webhook, "https://discord.com/api/webhooks/123/abc");
    }

    #[test]
    fn cli_parses_all_options() {
        let cli = Cli::try_parse_from([
            "discord-pipe",
            "--webhook", "https://discord.com/api/webhooks/123/abc",
            "--tag", "my-build",
            "--window-ms", "3000",
            "--max-lines", "100",
            "--max-bytes", "1500",
            "--max-messages", "5",
            "--format", "embed",
            "--username", "BuildBot",
            "--no-strip-ansi",
            "--dry-run",
            "--quiet",
        ]).unwrap();
        assert_eq!(cli.tag, "my-build");
        assert_eq!(cli.window_ms, 3000);
        assert_eq!(cli.max_lines, 100);
        assert_eq!(cli.max_bytes, 1500);
        assert_eq!(cli.max_messages, 5);
        assert_eq!(cli.format, Format::Embed);
        assert_eq!(cli.username.as_deref(), Some("BuildBot"));
        assert!(cli.no_strip_ansi);
        assert!(cli.dry_run);
        assert!(cli.quiet);
    }

    #[test]
    fn cli_parses_follow_flag() {
        let cli = Cli::try_parse_from([
            "discord-pipe",
            "--webhook", "https://discord.com/api/webhooks/123/abc",
            "--follow", "/var/log/app.log",
        ]).unwrap();
        assert_eq!(cli.follow.as_deref(), Some(std::path::Path::new("/var/log/app.log")));
    }

    #[test]
    fn cli_defaults() {
        let cli = Cli::try_parse_from([
            "discord-pipe",
            "--webhook", "https://discord.com/api/webhooks/123/abc",
        ]).unwrap();
        assert_eq!(cli.tag, "discord-pipe");
        assert_eq!(cli.window_ms, 2000);
        assert_eq!(cli.max_lines, 50);
        assert_eq!(cli.max_bytes, 1800);
        assert_eq!(cli.max_messages, 3);
        assert_eq!(cli.format, Format::Code);
        assert!(!cli.no_strip_ansi);
        assert!(!cli.dry_run);
        assert!(!cli.quiet);
    }
}
```

- [ ] Write tests -> `cargo test` -> confirm fail
- [ ] Implement `Cli` struct with `clap::Parser` derive:
  - `--webhook` / `DISCORD_WEBHOOK_URL` env var (required)
  - `--tag` (default: "discord-pipe")
  - `--follow <PATH>` (optional, mutually informational with stdin)
  - `--window-ms` (default: 2000)
  - `--max-lines` (default: 50)
  - `--max-bytes` (default: 1800)
  - `--max-messages` (default: 3)
  - `--format` enum: `Code`, `Embed` (default: Code)
  - `--username` (optional)
  - `--no-strip-ansi` (flag, default false)
  - `--dry-run` (flag)
  - `--quiet` (flag)
- [ ] `cargo test` -> confirm pass
- [ ] Commit: `feat: add CLI argument parsing`

---

### Phase 9: Main Loop Integration

#### Step 12: Wire up the main loop (producer thread)

- [ ] Implement `run(cli: Cli) -> Result<(), Box<dyn Error>>`:
  - Validate webhook URL format (starts with `https://discord.com/api/webhooks/` or
    `https://discordapp.com/api/webhooks/`)
  - Create bounded `mpsc::sync_channel(50)` for batch queue
  - Spawn sender thread (Step 13)
  - If `--follow` is set: spawn `tail_file` sending lines to a line channel
  - Otherwise: read from stdin via `LineReader`
  - Main loop:
    - Receive lines, push to `BatchBuffer`
    - Check time window (track last flush time with `Instant`)
    - When `should_flush()` OR time window expired OR EOF: drain batch, send to queue
  - On EOF: drain final batch, send sentinel, join sender thread
- [ ] Test with `--dry-run` mode (prints to stdout instead of posting)
- [ ] Commit: `feat: wire up main producer loop`

#### Step 13: Wire up the sender thread (consumer)

- [ ] Implement sender thread loop:
  - Pop `Batch` from channel (blocking recv)
  - Apply rate limiting (wait if bucket empty)
  - Format batch (code block or embed)
  - Split if oversized
  - Post each chunk via `UreqPoster`
  - Sync rate limit bucket from response headers
  - On error: retry with backoff (1s, 2s, 4s), max 3 attempts
  - On permanent error (401/404): log, signal main thread to shut down
  - Track and report dropped messages
  - On sentinel (EOF): exit loop
- [ ] Commit: `feat: wire up sender consumer thread`

#### Step 14: Signal handling

- [ ] Register `ctrlc` handler that:
  - Sets an `AtomicBool` shutdown flag
  - Main thread checks flag each iteration, flushes remaining batch on shutdown
  - Sender thread drains queue on shutdown (bounded: max 10s)
- [ ] Test: `echo "test" | cargo run -- --webhook $URL --dry-run` should exit cleanly
- [ ] Commit: `feat: add graceful shutdown on SIGINT/SIGTERM`

---

### Phase 10: Integration Tests

#### Step 15: End-to-end dry-run test

**Test** (`tests/integration.rs`):
```rust
use std::process::Command;

#[test]
fn dry_run_batches_stdin_to_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args(["--webhook", "https://discord.com/api/webhooks/fake/fake", "--dry-run", "--tag", "test"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(b"line1\nline2\nline3\n").unwrap();
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("line1"));
    assert!(stdout.contains("line2"));
    assert!(stdout.contains("line3"));
    assert!(stdout.contains("```")); // code block format
    assert!(stdout.contains("[test]")); // tag
}

#[test]
fn dry_run_strips_ansi_by_default() {
    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args(["--webhook", "https://discord.com/api/webhooks/fake/fake", "--dry-run"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(b"\x1b[31mred\x1b[0m\n").unwrap();
            child.wait_with_output()
        })
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("red"));
    assert!(!stdout.contains("\x1b["));
}

#[test]
fn exits_with_error_on_missing_webhook() {
    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();

    assert!(!output.status.success());
}
```

- [ ] Write integration tests -> `cargo test` -> confirm fail (binary not fully wired)
- [ ] Fix any integration issues
- [ ] `cargo test` -> all pass
- [ ] Commit: `test: add end-to-end integration tests`

#### Step 16: Dry-run file tail integration test

```rust
#[test]
fn dry_run_tails_file() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("test.log");
    std::fs::write(&log_path, "").unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args([
            "--webhook", "https://discord.com/api/webhooks/fake/fake",
            "--dry-run",
            "--follow", log_path.to_str().unwrap(),
            "--window-ms", "500",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    // Append lines to the file
    std::thread::sleep(Duration::from_millis(200));
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(&log_path).unwrap();
    writeln!(f, "tailed line 1").unwrap();
    writeln!(f, "tailed line 2").unwrap();

    // Wait for batch window to flush
    std::thread::sleep(Duration::from_millis(1000));

    // Kill the process and check output
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("tailed line 1"));
    assert!(stdout.contains("tailed line 2"));
}
```

- [ ] Write test -> `cargo test` -> confirm fail or pass
- [ ] Fix any issues
- [ ] Commit: `test: add file tail integration test`

---

### Phase 11: Polish & Release Prep

#### Step 17: README and documentation

- [ ] Write `README.md` with:
  - One-line description
  - Installation (`cargo install`)
  - Usage examples (stdin pipe, file tail, dry-run)
  - All CLI flags documented
  - Environment variables
  - Rate limit behavior explanation
  - License
- [ ] Update `CLAUDE.md` to remove WASM references
- [ ] Commit: `docs: add README and update project docs`

#### Step 18: Final verification

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --workspace -- -D warnings` - zero warnings
- [ ] `cargo test --workspace` - all pass
- [ ] Test manually: `echo "hello world" | cargo run -- --webhook $URL --dry-run`
- [ ] Test manually: `cargo run -- --webhook $URL --dry-run --follow /tmp/test.log` (append to file in another terminal)
- [ ] Commit any final fixes
- [ ] Tag: `v0.1.0`
- [ ] Commit: `chore: prepare v0.1.0 release`

---

## Files Affected

| File | Change |
|------|--------|
| `Cargo.toml` | New - project manifest with dependencies |
| `src/main.rs` | New - entry point, CLI parsing, main loop |
| `src/ansi.rs` | New - ANSI escape code stripping |
| `src/batcher.rs` | New - batch buffer with flush triggers |
| `src/format.rs` | New - code block + embed formatting, overflow splitting |
| `src/sender.rs` | New - HTTP poster, token bucket, retry logic |
| `src/reader.rs` | New - stdin reader + file tail |
| `tests/integration.rs` | New - end-to-end tests |
| `README.md` | New - project documentation |
| `CLAUDE.md` | Updated - remove WASM references |

## Dependency Budget

| Crate | Version | Purpose |
|-------|---------|---------|
| `clap` | latest (derive) | CLI argument parsing |
| `ureq` | latest | Blocking HTTP client |
| `serde` | latest | Serialization framework |
| `serde_json` | latest | JSON serialization for webhook payloads |
| `ctrlc` | latest | Signal handling (SIGINT/SIGTERM) |
| `dotenvy` | latest | `.env` file loading |
| `notify` | latest | Filesystem events for file tailing |
| `tempfile` | latest (dev) | Temporary files for tests |

## Testing Plan

### Unit Tests (per module)
- `ansi.rs`: 7 tests covering color codes, cursor movement, edge cases
- `batcher.rs`: 6 tests covering accumulation, thresholds, Unicode counting
- `format.rs`: 4+ tests for code block format, embed format, overhead calculation
- `format.rs` (splitting): 4 tests for single chunk, multi-chunk, middle truncation, line boundaries
- `sender.rs` (bucket): 5 tests for token lifecycle, draining, header sync
- `sender.rs` (sender): 4+ tests for success, 429 retry, 5xx retry, permanent error
- `reader.rs` (stdin): 5 tests for line reading, empty input, ANSI stripping
- `reader.rs` (tail): 3 tests for append detection, start position, file creation
- `main.rs` (cli): 5 tests for argument parsing, defaults, validation

### Integration Tests
- Dry-run stdin pipeline (format verification)
- Dry-run ANSI stripping
- Missing webhook error exit
- Dry-run file tail

### Manual Verification
- Real Discord webhook posting (not automated - requires live webhook)
- Long-running tail test
- Rate limit behavior under load

## ADR Required?

No. The architectural decisions were made in the brainstorm doc and approved. No new
decisions are introduced in this plan.

## Risks / Open Questions

1. **`notify` crate reliability** - filesystem event libraries can be flaky across
   platforms. The polling fallback helps, but file tail tests may be timing-sensitive.
   Mitigation: use generous timeouts in tests, document known flaky scenarios.

2. **Token bucket accuracy** - the local bucket will drift from Discord's actual state
   between syncs. The header sync corrects this, but there's a window where we might
   slightly over- or under-send. Mitigation: this is acceptable; the 429 retry handles
   the worst case.

3. **Single-line > 2000 chars** - a single input line longer than 2000 Unicode codepoints
   will need truncation even after splitting. The `split_content` function should handle
   this edge case (truncate the individual line with `... (truncated)`).

4. **Test timing sensitivity** - file tail tests rely on timing (sleep + filesystem events).
   May be flaky in CI. Mitigation: generous timeouts, retry-on-timeout in CI if needed.

5. **`mpsc::sync_channel` back-pressure** - if the sender can't keep up (Discord down +
   queue full), `sync_channel` will block the producer. This is intentional - it provides
   back-pressure rather than unbounded memory growth. But it means stdin reading will stall.
   The bounded channel capacity of 50 batches at ~2KB each caps memory at ~100KB.
