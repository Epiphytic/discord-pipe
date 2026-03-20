# Brainstorm: discord-pipe Design

**Date:** 2026-03-20
**Status:** Draft - awaiting review

## Problem Statement

Long-running CLI tools (builds, tests, deployments, AI agents) produce output faster than Discord's webhook rate limit allows. Naively piping `stdout`/`stderr` to a Discord webhook results in HTTP 429 responses and dropped messages.

We need a **generic, single-binary CLI tool** written in Rust that:

1. Ingests output from various sources (stdin pipe, named pipe, file tail)
2. Batches that output intelligently
3. Posts to Discord webhooks while respecting rate limits
4. Optionally compiles to a WASM component for sandboxed execution

**Assumptions:**
- Webhook-only (no bot token, no OAuth, no message editing/threading)
- Append-only message stream (no updating previous messages)
- Single webhook target per invocation (multiple webhooks = multiple processes)
- Primary use case: `my-cli 2>&1 | discord-pipe --webhook $URL`
- Must work on Linux and macOS at minimum

---

## 1. Input Sources

### Option A: Stdin-Only (Minimal)

**Mechanism:** Read from stdin using `BufReader<Stdin>`, line by line. Classic Unix pipe pattern.

**Optimized for:** Simplicity, composability, WASM compatibility.

**Drawbacks:**
- Can't attach to already-running processes (need to start them with the pipe)
- Only one input stream per process (can't merge stdout + stderr from different sources)
- No way to "reconnect" if discord-pipe crashes

**Fit:** Perfect for the primary use case (`cmd 2>&1 | discord-pipe`). This is the Unix way and covers 80%+ of use cases. WASM variant can support this too since WASI provides stdin.

### Option B: Named Pipe (FIFO)

**Mechanism:** Create or open a FIFO at a specified path. Multiple writers can push data to it. discord-pipe reads from it like a file.

**Optimized for:** Decoupled producer/consumer, multiple writers.

**Drawbacks:**
- Platform-specific (Unix only, no native Windows FIFOs)
- FIFO semantics are tricky: reader blocks until a writer opens, writer blocks if no reader. If all writers close and reopen, reader sees EOF and must reopen.
- Requires lifecycle management (who creates/cleans up the FIFO?)
- Not WASM-compatible (no filesystem pipes in WASI)
- Adds complexity for a niche use case

**Fit:** Useful for long-running daemons or multi-source aggregation, but significant complexity cost. Could be added later as a feature flag.

### Option C: File Tail (like `tail -f`)

**Mechanism:** Watch a file path, read new bytes as they're appended. Use `notify` crate for filesystem events with polling fallback.

**Optimized for:** Attaching to log files from already-running processes. Works with tools that write to files instead of stdout.

**Drawbacks:**
- File rotation handling (logrotate, truncation) adds complexity
- Polling fallback on some filesystems (NFS, FUSE) adds latency
- inotify/kqueue dependency
- Must handle: file doesn't exist yet, file is truncated, file is replaced
- Not WASM-compatible
- Race condition at startup: where to start reading? End of file? Beginning?

**Fit:** Genuinely useful for tailing CI log files or daemon logs. But it's a secondary mode with significant edge cases.

### Option D: Hybrid - Stdin Primary + File Tail Feature-Flagged

**Mechanism:** Stdin is the default and always-supported mode. File tail is behind a `--follow <path>` flag, compiled with a `tail` feature flag. Named pipe support is not included initially.

**Optimized for:** Covering the two most common use cases without over-engineering.

**Drawbacks:**
- Two code paths to maintain
- Feature flag increases test matrix

**Fit:** Best balance. Ship stdin first, add `--follow` when/if needed.

### Recommendation: Option D (Hybrid), but ship stdin-only for v0.1

Start with stdin as the sole input. It covers the primary use case, is WASM-compatible, and is dead simple. Named pipe and file tail can be added as feature-flagged modes in later versions if demand materializes. Don't build what we don't need yet.

---

## 2. Batching Strategy

### Option A: Time-Window Only

**Mechanism:** Buffer all incoming lines. Every N milliseconds, flush the buffer as a single Discord message. Timer resets after each flush.

**Optimized for:** Predictable posting cadence. Simple implementation.

**Drawbacks:**
- If output is slow (1 line per 10s), you wait the full window before posting. Feels laggy.
- If output is extremely fast, a single window could accumulate more text than Discord's 2000-char limit, requiring splitting anyway.
- No backpressure from message size.

**Fit:** Works but feels dumb. The window either has to be short (frequent posts, hits rate limit) or long (delayed output, bad UX).

### Option B: Line-Count Only

**Mechanism:** Buffer lines. When count reaches N, flush immediately.

**Optimized for:** Predictable message sizes. Works well when output is flowing steadily.

**Drawbacks:**
- If output stalls (build waiting for download), accumulated lines sit in buffer indefinitely. User never sees partial output.
- Doesn't respect rate limits directly - fast output triggers rapid flushes.

**Fit:** Must be combined with time-based flushing to avoid the stall problem.

### Option C: Hybrid - Time Window + Line Count + Size Limit

**Mechanism:** Flush the buffer when ANY of these conditions is met:
1. **Time window expires** (e.g., 2 seconds since last flush)
2. **Line count threshold** (e.g., 50 lines accumulated)
3. **Byte size threshold** (e.g., approaching 1900 bytes to leave room for formatting)
4. **EOF/stream close** (flush remaining buffer)

Additionally, a **minimum debounce** prevents flushing more than once per ~400ms (respecting the 5 req/2s rate limit).

**Optimized for:** Responsiveness AND efficiency. Slow output posts quickly (time window). Fast output batches aggressively (line count/size). Never exceeds Discord limits.

**Drawbacks:**
- Three tuning knobs instead of one
- Edge case: what if a single line exceeds 2000 chars? Need truncation or splitting.

**Fit:** This is the correct answer. The question is what defaults to pick.

### Option D: Adaptive Window

**Mechanism:** Like Option C, but the time window adapts based on output rate. Fast output = longer windows (more batching). Slow output = shorter windows (more responsive).

**Optimized for:** Automatic tuning. "Just works."

**Drawbacks:**
- Harder to reason about behavior
- More complex implementation
- Users can't predict when messages will appear
- Harder to test

**Fit:** Over-engineered for v1. The static hybrid approach with sensible defaults is good enough.

### Proposed Defaults for Option C

| Parameter | Default | Flag | Rationale |
|-----------|---------|------|-----------|
| Time window | 2s | `--window-ms 2000` | Posts at most every 2s during slow output. Leaves headroom under 5/2s rate limit. |
| Max lines | 50 | `--max-lines 50` | ~50 lines of typical terminal output fits comfortably in 2000 chars |
| Max bytes | 1800 | `--max-bytes 1800` | 2000 char limit minus formatting overhead (code block markers, tag line) |
| Min interval | 400ms | (internal) | Hard floor: never post faster than 5/2s = 400ms apart |

### Recommendation: Option C with the above defaults

Static hybrid batching. Three flush triggers (time, lines, bytes), one hard rate-limit floor. Simple, predictable, covers all output patterns.

---

## 3. WASM Variant Feasibility

### Current State of wasi:http

- **wasi:http** is Phase 3 (implementation phase) as of the latest WASI spec
- The repository has been archived into the main WebAssembly/WASI repo (Nov 2025)
- `wasi:http/outgoing-handler` is the relevant interface for making outbound HTTP requests
- **Wasmtime** fully supports `wasi:http` via `wasmtime-wasi-http` crate
- **Spin** framework supports outbound HTTP via the same interface
- The WIT interface provides: `outgoing-handler::handle(request) -> future<response>`

### Option A: Full WASM Component (wasm32-wasip2)

**Mechanism:** Compile discord-pipe as a WASM component targeting `wasm32-wasip2`. Use `wasi:http/outgoing-handler` for Discord webhook calls. Read stdin via `wasi:cli/stdin`. Timer/batching via `wasi:clocks/monotonic-clock`.

**WIT world definition:**
```wit
package discord:pipe@0.1.0;

world discord-pipe {
    import wasi:cli/stdin@0.2.0;
    import wasi:cli/stdout@0.2.0;
    import wasi:cli/stderr@0.2.0;
    import wasi:http/outgoing-handler@0.2.0;
    import wasi:clocks/monotonic-clock@0.2.0;
    import wasi:cli/environment@0.2.0;

    export wasi:cli/run@0.2.0;
}
```

**Optimized for:** Sandboxed execution, capability-based security (host controls which URLs are reachable), portability.

**Drawbacks:**
- stdin-only (no named pipes, no file tail in WASI)
- `wasi:http` requires the host to grant HTTP capability (e.g., Wasmtime `--allow-http`)
- Async in WASI 0.2 is resource-based and can be awkward; WASI 0.3 improves this
- Binary size will be larger due to component model overhead
- Ecosystem: `reqwest` and `ureq` don't compile to `wasm32-wasip2` directly - need to use the raw WIT bindings or a WASI-aware HTTP crate
- Can't use `tokio` runtime in WASM; need single-threaded event loop or blocking calls
- Testing is harder (need Wasmtime or similar host to run)

**Fit:** Technically feasible today with Wasmtime. The main challenge is the HTTP client - you'd need to use raw WIT bindings (via `wit-bindgen`) rather than ergonomic Rust HTTP crates. Timer handling for batching requires `wasi:clocks`.

### Option B: Shared Core Library + Platform-Specific I/O

**Mechanism:** Factor the batching logic, formatting, and message construction into a `#[no_std]`-compatible core library. Native binary uses `reqwest`/`ureq` + `tokio`. WASM binary wraps the same core with `wasi:http` bindings.

**Optimized for:** Code reuse, clean separation of concerns, testable core.

**Drawbacks:**
- More architectural complexity upfront
- `#[no_std]` constraints may be painful for string/collection operations (need `alloc`)
- Two build targets to maintain and test

**Fit:** The right architecture if we're serious about WASM. But it's premature for v0.1.

### Option C: Native-Only, WASM Later

**Mechanism:** Build the native binary first. Don't architect for WASM upfront, but keep the code modular enough that extracting a core library later is feasible.

**Optimized for:** Shipping quickly, avoiding premature abstraction.

**Drawbacks:**
- May require refactoring when WASM variant is actually built
- Risk of coupling to native-only APIs

**Fit:** Pragmatic. The WASM use case is speculative - build for it when there's a concrete consumer.

### Feasibility Assessment

| Capability | WASI 0.2 Support | Notes |
|------------|-------------------|-------|
| Read stdin | Yes (`wasi:cli/stdin`) | Line-buffered reading works |
| HTTP POST | Yes (`wasi:http/outgoing-handler`) | Host must grant capability |
| Timers | Yes (`wasi:clocks/monotonic-clock`) | Polling-based, not async timers |
| Env vars | Yes (`wasi:cli/environment`) | For webhook URL config |
| File I/O | Partial (`wasi:filesystem`) | Only pre-opened dirs, no arbitrary paths |
| Named pipes | No | Not in WASI spec |
| Async/tokio | No | Single-threaded, cooperative |

**Verdict:** WASM variant is feasible for stdin-only mode. The `wasi:http/outgoing-handler` interface can POST to Discord webhooks. The main pain point is the HTTP client - you'd use `wit-bindgen` generated bindings directly rather than `reqwest`.

### Recommendation: Option C - native first, WASM later

Build the native binary. Keep I/O and HTTP behind trait abstractions (good practice anyway). Circle back to WASM when there's a concrete use case (e.g., running inside a GIRT tool).

---

## 4. Discord API Constraints & Rate Limit Handling

### Hard Facts

| Constraint | Value | Source |
|------------|-------|--------|
| Rate limit | 5 requests per 2 seconds per webhook | Discord docs, confirmed by community |
| Message content limit | 2000 characters | Discord API |
| Embed description limit | 4096 characters | Discord API |
| Total embed limit | 6000 characters across all embeds | Discord API |
| Max embeds per message | 10 | Discord API |
| Rate limit scope | Per webhook URL (webhook_id + token) | Each webhook has independent limits |
| Rate limit headers | `X-RateLimit-Remaining`, `X-RateLimit-Reset-After` | Included in every response |
| 429 response | Includes `Retry-After` header (seconds) | Must honor this |

### Option A: Proactive Rate Limiting (Token Bucket)

**Mechanism:** Implement a local token bucket rate limiter. 5 tokens, refill every 2 seconds. Never send a request if no tokens available. Queue messages internally.

**Optimized for:** Never hitting 429. Predictable behavior. No wasted requests.

**Drawbacks:**
- Doesn't account for other clients using the same webhook
- Clock drift between local timer and Discord's rate limit window
- If another tool is posting to the same webhook, our local bucket is wrong

**Fit:** Good as the primary strategy, but must be combined with 429 handling as a fallback.

### Option B: Reactive Rate Limiting (Response Headers)

**Mechanism:** Send requests freely. Read `X-RateLimit-Remaining` from each response. When remaining hits 0, wait `X-RateLimit-Reset-After` seconds. On 429, wait `Retry-After` and retry.

**Optimized for:** Accuracy. Discord tells us exactly when we can send again.

**Drawbacks:**
- First 429 means we already "wasted" a request
- Slightly more complex state management
- Requires parsing response headers on every call

**Fit:** Essential as a fallback, but shouldn't be the only strategy.

### Option C: Hybrid - Token Bucket + Response Headers (Recommended)

**Mechanism:**
1. Local token bucket: 5 tokens / 2 seconds. Never send without a token.
2. After each request, update the bucket from `X-RateLimit-Remaining` and `X-RateLimit-Reset-After` headers. This self-corrects the local bucket against Discord's actual state.
3. On 429: log a warning, read `Retry-After`, sleep, retry once. If retry also fails, queue the message and move on.
4. Internal message queue with bounded size (e.g., 100 messages). If queue fills, drop oldest with a "messages dropped" indicator.

**Optimized for:** Never hitting 429 under normal conditions, graceful recovery when it does happen, resilience against other webhook consumers.

**Fit:** This is the industry-standard approach. Not complex to implement.

### Overflow Handling (Message > 2000 chars)

When a batched message exceeds 2000 characters, we have several choices:

**Strategy 1: Split into multiple messages**
- Split on line boundaries, keeping each chunk under 1800 chars (room for formatting)
- Each chunk gets its own code block markers
- Produces readable output
- Consumes multiple rate limit tokens per batch

**Strategy 2: Truncate with indicator**
- Hard truncate at ~1800 chars
- Append `\n... (X lines truncated)` 
- Single message per batch
- Loses information

**Strategy 3: Hybrid split with cap**
- Split into up to 3 messages per batch (configurable)
- If content exceeds 3 messages worth, truncate the middle with `... (X lines omitted)`
- Balance between completeness and rate limit consumption

### Recommendation: Strategy 3 (Hybrid split with cap)

Split oversized batches into up to `--max-messages 3` Discord messages. If content still exceeds that, truncate from the middle (keep first and last chunks - the beginning shows what started, the end shows where we are). This preserves the most useful context while bounding rate limit consumption.

---

## 5. Output Formatting

### Option A: Plain Text

**Mechanism:** Post raw text as `content` field.

```
[cargo build] 2026-03-20 07:15:00
   Compiling discord-pipe v0.1.0
   Compiling serde v1.0.200
warning: unused variable `x`
```

**Optimized for:** Simplicity.

**Drawbacks:** No syntax highlighting. Long lines wrap awkwardly in Discord. Hard to distinguish from normal chat.

### Option B: Code Blocks

**Mechanism:** Wrap output in triple-backtick code blocks.

````
```
[cargo build] 2026-03-20 07:15:00
   Compiling discord-pipe v0.1.0
   Compiling serde v1.0.200
warning: unused variable `x`
```
````

**Optimized for:** Readability. Monospace font. Visual separation from chat.

**Drawbacks:** Code block markers consume ~8 chars of the 2000-char budget. Can't use markdown inside the block.

### Option C: Code Blocks with Language Hint

**Mechanism:** Like B, but with a language hint for syntax highlighting.

````
```ansi
[cargo build] 2026-03-20 07:15:00
   Compiling discord-pipe v0.1.0
   Compiling serde v1.0.200
warning: unused variable `x`
```
````

Discord supports `ansi` as a language hint, which renders ANSI color codes. Other useful hints: `bash`, `rust`, `diff`.

**Optimized for:** Colored output in Discord. ANSI escape codes from cargo/gcc/etc. are rendered.

**Drawbacks:** ANSI code support in Discord is limited. Not all escape codes render correctly. Adds 4-8 bytes for the language hint.

### Option D: Embeds

**Mechanism:** Use Discord embed objects instead of `content`. Embed description supports 4096 chars (2x the content limit).

```json
{
  "embeds": [{
    "title": "cargo build",
    "description": "```\n...\n```",
    "color": 3066993,
    "timestamp": "2026-03-20T07:15:00Z"
  }]
}
```

**Optimized for:** More characters (4096 in description). Visual polish with colors, titles, timestamps. Structured metadata.

**Drawbacks:**
- More complex JSON payload
- Embeds have their own size limits (6000 total across all embeds)
- Some webhook consumers may not render embeds the same way
- Embed description still needs code blocks for monospace

### Formatting Structure

Regardless of format choice, each message should include:

1. **Tag/label** - identifies the source (`--tag "cargo build"`)
2. **Timestamp** - when the batch was collected (not posted)
3. **Content** - the actual output
4. **Sequence indicator** - `[2/3]` if message was split
5. **Truncation indicator** - `... (47 lines omitted)` if applicable

### Recommendation: Option B (Code Blocks) as default, Option D (Embeds) as `--format embed` flag

Code blocks are the simplest, most readable default. They work universally, render monospace, and are easy to copy-paste. Embeds are a nice-to-have for users who want structured metadata and the extra character budget.

Default format template:
````
**`[{tag}]`** {timestamp}
```
{content}
```
````

The tag and timestamp sit outside the code block (using Discord markdown bold + inline code). The output goes inside the code block. This uses ~40 chars for overhead, leaving ~1960 for content.

---

## 6. Error Handling & Resilience

### What can go wrong?

1. **Discord is down (5xx)** - temporary outage
2. **Webhook URL is invalid/deleted (401/404)** - permanent failure
3. **Rate limited (429)** - temporary, self-correcting
4. **Network unreachable** - DNS failure, no internet
5. **discord-pipe gets SIGTERM/SIGINT** - graceful shutdown needed
6. **stdin closes unexpectedly** - upstream process crashed
7. **Message payload is malformed** - edge case in content

### Option A: Fire-and-Forget (Drop on Error)

**Mechanism:** Try to send. If it fails, log to stderr and move on. No retry, no buffering.

**Optimized for:** Simplicity. Never blocks the input pipeline.

**Drawbacks:** Silent data loss. No way to know messages were dropped without watching stderr.

### Option B: Retry with Bounded Queue

**Mechanism:**
- On transient failure (5xx, 429, network error): queue the message, retry after backoff
- Bounded queue (e.g., 100 messages, ~200KB). If full, drop oldest.
- On permanent failure (401, 404): log error, exit with non-zero code
- Exponential backoff: 1s, 2s, 4s, 8s, max 30s
- Max retries per message: 3

**Optimized for:** Resilience without unbounded memory growth.

**Drawbacks:**
- Queue can delay output during extended outages
- If Discord is down for minutes, queue fills and starts dropping anyway
- More complex state machine

### Option C: Retry + Disk Spill

**Mechanism:** Like B, but when the in-memory queue fills, spill to a temporary file. On recovery, drain the file first.

**Optimized for:** Zero message loss during extended outages.

**Drawbacks:**
- Significant complexity
- Disk I/O in a pipe tool feels wrong
- File cleanup on crash
- Not WASM-compatible
- The output is ephemeral CLI output - is it really worth persisting?

### Option D: Retry + Status Reporting (Recommended)

**Mechanism:**
- Transient errors: retry up to 3 times with exponential backoff (1s, 2s, 4s)
- Bounded in-memory queue: 50 messages max
- When queue is full: drop oldest, increment drop counter
- Periodic status messages to Discord: "discord-pipe: X messages dropped due to rate limiting/errors"
- Permanent errors (401/404): print to stderr, exit code 1 after flushing remaining buffer
- On SIGINT/SIGTERM: flush current buffer (one final send attempt), then exit cleanly
- On stdin EOF: flush buffer, wait for queue to drain (up to 10s), then exit

**Optimized for:** Transparency. Users know when messages are being dropped. Bounded resource usage.

### Recommendation: Option D

The key insight is that this tool pipes ephemeral CLI output - it's not a message queue. If Discord is down for 5 minutes during a build, it's fine to drop the middle and report the gap. What matters is:
1. Never block the input pipeline (always keep reading stdin)
2. Never use unbounded memory
3. Tell the user when messages are lost
4. Clean shutdown on signals

---

## 7. HTTP Client Choice (Native Binary)

### Option A: reqwest (async)

**Pros:** Most popular Rust HTTP crate. Async-native. Connection pooling. Well-maintained.
**Cons:** Heavy dependency tree (hyper, h2, tower, etc.). Pulls in tokio. Overkill for simple POST requests.
**Binary size impact:** ~3-5MB additional

### Option B: ureq (sync/blocking)

**Pros:** Simple API. Pure Rust. Small dependency tree. No async runtime needed. Forbids `unsafe`.
**Cons:** Blocking I/O - need a separate thread for HTTP if reading stdin on main thread. No connection pooling (fine for 5 req/2s).
**Binary size impact:** ~1-2MB additional

### Option C: reqwest with minimal features

**Pros:** Async without full hyper stack by using `rustls-tls` only. Can disable unused features.
**Cons:** Still pulls in tokio runtime.

### Recommendation: ureq for v0.1

For a tool that makes at most 2.5 HTTP POSTs per second to a single endpoint, `ureq` is the right choice. It's simple, safe, small, and doesn't require an async runtime. The architecture is: main thread reads stdin and batches, a sender thread pops from a queue and does blocking HTTP via `ureq`. Two threads, no async, minimal complexity.

If we later need async (e.g., multiple webhooks, or WASM compat), we can swap to `reqwest` or raw `wasi:http`.

---

## 8. CLI Interface Design

```
discord-pipe 0.1.0
Batch CLI output to Discord webhooks

USAGE:
    discord-pipe [OPTIONS]

OPTIONS:
    -w, --webhook <URL>         Discord webhook URL (or DISCORD_WEBHOOK_URL env var)
    -t, --tag <TAG>             Label for messages (e.g., "cargo build") [default: "discord-pipe"]
        --window-ms <MS>        Batch time window in milliseconds [default: 2000]
        --max-lines <N>         Flush after N lines [default: 50]
        --max-bytes <N>         Flush at N bytes of content [default: 1800]
        --max-messages <N>      Max Discord messages per batch [default: 3]
        --format <FMT>          Output format: text, code, embed [default: code]
        --username <NAME>       Override webhook display name
        --dry-run               Print batched messages to stdout instead of posting
        --quiet                 Suppress discord-pipe's own stderr logging
    -h, --help                  Print help
    -V, --version               Print version
```

Config precedence: CLI flags > env vars > defaults.

Environment variables:
- `DISCORD_WEBHOOK_URL` - webhook URL
- `DISCORD_PIPE_TAG` - default tag
- `DISCORD_PIPE_WINDOW_MS` - batch window

---

## Unknowns

- [ ] **Embed character limits in practice** - docs say 4096 for description, but need to verify with actual Discord webhook calls (some limits are undocumented or changed)
- [ ] **ANSI escape code handling** - should discord-pipe strip ANSI codes by default? Pass them through (Discord renders some)? Make it configurable?
- [ ] **Unicode handling** - Discord's 2000 char limit is in Unicode codepoints, not bytes. Need to verify how multi-byte characters are counted.
- [ ] **Webhook URL validation** - should we validate the URL format at startup, or just let the first POST fail?
- [ ] **Multiple webhook targets** - is there a use case for posting the same output to multiple webhooks? Out of scope for v0.1 but worth noting.
- [ ] **Signal handling on Windows** - how do SIGINT/SIGTERM map? Is `Ctrl+C` handling sufficient?
- [ ] **WASM HTTP client story** - when we get to the WASM variant, which crate/binding approach is most ergonomic? `wit-bindgen` raw bindings vs. a WASI-native HTTP crate?
- [ ] **Discord's undocumented per-channel rate limit** - some sources mention a per-channel limit shared across all webhook senders. Need to validate whether this affects us.

---

## Architecture Sketch

```
                     ┌──────────────────────────────────────────┐
                     │             discord-pipe                  │
                     │                                          │
  stdin ──────────►  │  ┌─────────┐    ┌──────────┐    ┌─────┐ │
  (or future:        │  │ Reader  │───►│ Batcher  │───►│Queue│ │
   --follow file)    │  │ (main   │    │ (time +  │    │(50) │ │
                     │  │  thread)│    │  lines + │    │     │ │
                     │  └─────────┘    │  bytes)  │    └──┬──┘ │
                     │                 └──────────┘       │    │
                     │                                    ▼    │
                     │                 ┌──────────────────────┐ │
                     │                 │ Sender Thread        │ │
                     │                 │ - token bucket       │ │  ──► Discord
                     │                 │ - ureq HTTP POST     │ │      Webhook
                     │                 │ - retry logic        │ │
                     │                 │ - header parsing      │ │
                     │                 └──────────────────────┘ │
                     │                                          │
                     └──────────────────────────────────────────┘
```

**Threading model:** Two threads.
1. **Main thread:** reads stdin line-by-line, accumulates into batch buffer, flushes to queue when batch triggers fire (time/lines/bytes).
2. **Sender thread:** pops from queue, applies rate limiting, posts to Discord, handles retries.

Communication: `crossbeam-channel` or `std::sync::mpsc` bounded channel (capacity = 50 messages).

---

## Dependency Budget (v0.1 native)

| Crate | Purpose | Weight |
|-------|---------|--------|
| `clap` (derive) | CLI arg parsing | Medium |
| `ureq` | HTTP client | Light |
| `serde` + `serde_json` | JSON serialization for webhook payload | Medium |
| `chrono` or `time` | Timestamps | Light (prefer `time` for smaller footprint) |
| `ctrlc` | Signal handling | Tiny |
| `dotenvy` | .env file loading | Tiny |

**Notably absent:** `tokio`, `hyper`, `tracing` (use `eprintln!` for v0.1 - structured logging is overkill for a pipe tool).

---

## Summary of Recommendations

| Decision | Recommendation | Rationale |
|----------|---------------|-----------|
| Input source | Stdin-only for v0.1 | Unix pipe pattern, WASM compatible, covers primary use case |
| Batching | Hybrid: time (2s) + lines (50) + bytes (1800) | Responsive and efficient across all output rates |
| Rate limiting | Token bucket + response header correction + 429 retry | Industry standard, never hits 429 under normal use |
| Message overflow | Split up to 3 messages, truncate middle if still too large | Preserves context without burning rate limit budget |
| Formatting | Code blocks default, embeds optional | Readable, universal, simple |
| Error handling | Bounded retry + bounded queue + drop reporting | Transparent, bounded resources, never blocks stdin |
| HTTP client | ureq (blocking, two-thread model) | Simple, small, no async runtime needed |
| WASM | Defer to later version | Feasible but premature; keep code modular for extraction |
| CLI framework | clap derive | Standard, minimal config |
