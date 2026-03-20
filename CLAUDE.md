# discord-pipe

Single-binary Rust CLI that batches stdin or file-tail output and posts it to Discord webhooks.

## What it does
- Reads from stdin (pipe mode) or tails a file (`--follow`)
- Batches output by time window, line count, and character count
- Posts to Discord webhooks with rate limiting (token bucket + header sync + 429 retry)
- Supports code block and embed output formats
- Strips ANSI escape codes by default

## Build & test
```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

## Usage
```bash
my-cli 2>&1 | discord-pipe --webhook $DISCORD_WEBHOOK_URL --tag "my-build"
```

## Non-goals
- No Discord bot token (webhook only)
- No message editing/threading (append-only)
- Not tied to any specific CLI tool

## Constraints
- Single binary, no runtime deps
- Config via CLI flags + env vars + .env file
- MIT OR Apache-2.0 license
