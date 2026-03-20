# discord-pipe

Generic CLI output batcher that collates chatty stdout/stderr to Discord webhooks.

## Problem
Long-running CLI tools (jcode, cargo build, tests, deployments) produce output faster than Discord's 5 req/s rate limit. Naively piping causes 429s and dropped messages.

## Goal
A Rust binary (and optionally WASM component) that:
- Reads from stdin, a named pipe, or a file (tail mode)
- Batches output by time window AND line count
- Respects Discord rate limits
- Posts to a Discord webhook with configurable formatting
- Can be invoked generically: `my-cli 2>&1 | discord-pipe --webhook $URL --tag "cargo build"`

## Non-goals
- No Discord bot token (webhook only, simpler auth)
- No message editing/threading (append-only)
- Not jcode-specific

## Constraints
- Single binary, no runtime deps
- WASM variant: reads stdin only, uses wasi:http for Discord calls
- Config via CLI flags + optional .env / env vars
- Apache 2.0 or MIT license
