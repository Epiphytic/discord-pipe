# discord-pipe

Pipe CLI output to Discord webhooks with smart batching and rate limiting.

## Problem

Long-running CLI tools (builds, tests, deployments, log tails) produce output far faster than Discord's 5 requests/second webhook rate limit. Naively piping stdout to a webhook results in HTTP 429s and dropped messages.

`discord-pipe` sits between your CLI and Discord, batching output by time window, line count, and character count, then posting it as neatly formatted messages while respecting rate limits.

## Installation

```bash
# From source
cargo install --path .

# Or build manually
cargo build --release
# Binary at target/release/discord-pipe
```

## Usage

### Basic stdin pipe

```bash
my-cli 2>&1 | discord-pipe --webhook $DISCORD_WEBHOOK_URL
```

### With a tag label

```bash
cargo build 2>&1 | discord-pipe --webhook $URL --tag "cargo build"
```

### Tail a file

```bash
discord-pipe --webhook $URL --follow /var/log/app.log
```

### Dry-run (print to stdout instead of posting)

```bash
echo "test" | discord-pipe --webhook $URL --dry-run
```

### Embed format

```bash
my-cli | discord-pipe --webhook $URL --format embed
```

### Using an environment variable

```bash
export DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/123456/abcdef
my-cli 2>&1 | discord-pipe
```

## CLI Flags

| Flag | Default | Env Var | Description |
|------|---------|---------|-------------|
| `--webhook <URL>` | *(required)* | `DISCORD_WEBHOOK_URL` | Discord webhook URL |
| `--tag <TAG>` | `discord-pipe` | | Tag/label prepended to messages |
| `--follow <PATH>` | *(none)* | | Tail a file instead of reading stdin |
| `--window-ms <MS>` | `2000` | | Batch time window in milliseconds |
| `--max-lines <N>` | `50` | | Max lines per batch |
| `--max-bytes <N>` | `1800` | | Max characters per batch |
| `--max-messages <N>` | `3` | | Max Discord messages per batch overflow |
| `--format <FMT>` | `code` | | Output format: `code` or `embed` |
| `--username <NAME>` | *(none)* | | Discord webhook username override |
| `--no-strip-ansi` | `false` | | Don't strip ANSI escape codes from output |
| `--dry-run` | `false` | | Print formatted output to stdout instead of posting |
| `--quiet` | `false` | | Suppress status messages on stderr |

## How It Works

```
stdin/file --> [Line Reader] --> [Batch Buffer] --> [Sender] --> Discord Webhook
                   |                   |                |
              strip ANSI        time window +      token bucket +
              (default)        line count +       header sync +
                              char count          retry on 429
```

### Batching strategy

Lines are accumulated in a batch buffer. A batch is flushed when any of these conditions is met:

1. **Time window** (`--window-ms`): the batch has been open longer than the configured window (default 2s)
2. **Line count** (`--max-lines`): the batch reaches the max line count (default 50)
3. **Character count** (`--max-bytes`): the total characters in the batch reach the limit (default 1800)

### Overflow splitting

If a flushed batch exceeds Discord's 2000-character message limit, it is split into multiple messages up to `--max-messages` (default 3). Each chunk is tagged with a sequence indicator like `[1/3]`.

### Output formats

- **`code`** (default): wraps output in a Discord code block with a bold tag header and timestamp
- **`embed`**: posts output as a Discord embed with the tag as the title

### ANSI stripping

By default, ANSI escape codes (colors, bold, cursor movement) are stripped from input. Use `--no-strip-ansi` to preserve them.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `DISCORD_WEBHOOK_URL` | Discord webhook URL (alternative to `--webhook` flag) |

A `.env` file in the working directory is also loaded automatically.

## Rate Limit Behavior

`discord-pipe` uses a two-layer rate limiting strategy:

1. **Token bucket**: a local token bucket (5 tokens, refilling every 2 seconds) prevents bursting beyond Discord's global rate limit
2. **Discord header sync**: after each request, the `X-RateLimit-Remaining` and `X-RateLimit-Reset-After` headers are read to synchronize with Discord's actual rate limit state
3. **429 retry**: if Discord returns a 429 (rate limited), the `Retry-After` header is respected and the request is retried after the specified delay

This approach ensures reliable delivery without hitting Discord's rate limits even under heavy output.

## Architecture

Single-threaded reader, multi-threaded pipeline:

- **Reader thread**: reads lines from stdin or tails a file using filesystem notifications (`notify` crate)
- **Batcher**: accumulates lines in the main thread and flushes based on time/size thresholds
- **Sender thread**: receives batches over a channel, formats them, applies rate limiting, and posts to Discord

Graceful shutdown on Ctrl-C: in-flight batches are flushed before exit.

## License

MIT OR Apache-2.0
