use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;

mod ansi;
mod batcher;
mod format;
mod ndjson;
mod reader;
mod sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CliFormat {
    Code,
    Embed,
}

#[derive(Debug, clap::Parser)]
#[command(name = "discord-pipe", about = "Pipe CLI output to Discord webhooks")]
pub struct Cli {
    #[arg(long, env = "DISCORD_WEBHOOK_URL")]
    pub webhook: String,

    #[arg(long, default_value = "discord-pipe")]
    pub tag: String,

    #[arg(long)]
    pub follow: Option<std::path::PathBuf>,

    #[arg(long, default_value = "2000")]
    pub window_ms: u64,

    #[arg(long, default_value = "50")]
    pub max_lines: usize,

    #[arg(long, default_value = "1800")]
    pub max_bytes: usize,

    #[arg(long, default_value = "3")]
    pub max_messages: usize,

    #[arg(long, default_value = "code")]
    pub format: CliFormat,

    #[arg(long)]
    pub username: Option<String>,

    #[arg(long)]
    pub no_strip_ansi: bool,

    #[arg(long)]
    pub ndjson: bool,

    #[arg(long)]
    pub show_tool_calls: bool,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long)]
    pub quiet: bool,
}

#[allow(clippy::too_many_arguments)]
fn sender_thread(
    rx: std::sync::mpsc::Receiver<String>,
    webhook: &str,
    tag: &str,
    format: sender::Format,
    username: Option<&str>,
    dry_run: bool,
    quiet: bool,
    shutdown: Arc<AtomicBool>,
    max_messages: usize,
    max_bytes: usize,
) {
    if dry_run {
        for content in rx {
            let chunks = format::split_content(&content, max_bytes, max_messages);
            if chunks.len() == 1 {
                let formatted = match format {
                    sender::Format::Code => {
                        format::format_code_block(&content, tag, &sender::timestamp_now())
                    }
                    sender::Format::Embed => {
                        format::format_embed(&content, tag, &sender::timestamp_now())
                    }
                };
                println!("{formatted}");
            } else {
                for (i, chunk) in chunks.iter().enumerate() {
                    let seq_tag = std::format!("{tag} [{}/{}]", i + 1, chunks.len());
                    let formatted = match format {
                        sender::Format::Code => {
                            format::format_code_block(chunk, &seq_tag, &sender::timestamp_now())
                        }
                        sender::Format::Embed => {
                            format::format_embed(chunk, &seq_tag, &sender::timestamp_now())
                        }
                    };
                    println!("{formatted}");
                }
            }
        }
        return;
    }

    let bucket = sender::TokenBucket::new(5, Duration::from_secs(2));
    let poster = sender::UreqPoster;
    let mut s = sender::Sender::new(poster, webhook, bucket);
    let mut dropped = 0u64;

    for content in rx {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let chunks = format::split_content(&content, max_bytes, max_messages);
        for (i, chunk) in chunks.iter().enumerate() {
            let result = if chunks.len() == 1 {
                s.send_batch(chunk, tag, format, username)
            } else {
                let seq_tag = std::format!("{tag} [{}/{}]", i + 1, chunks.len());
                s.send_batch(chunk, &seq_tag, format, username)
            };

            if let Err(e) = result {
                dropped += 1;
                if !quiet {
                    eprintln!("discord-pipe: send failed: {e:?}");
                }
            }
        }
    }

    if dropped > 0 && !quiet {
        eprintln!("discord-pipe: {dropped} message(s) failed to send");
    }
}

fn main() {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();

    if !cli.dry_run
        && !cli.webhook.starts_with("https://discord.com/api/webhooks/")
        && !cli
            .webhook
            .starts_with("https://discordapp.com/api/webhooks/")
    {
        eprintln!(
            "error: webhook URL must start with https://discord.com/api/webhooks/ \
             or https://discordapp.com/api/webhooks/"
        );
        std::process::exit(1);
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_ctrlc = shutdown.clone();
    ctrlc::set_handler(move || {
        shutdown_ctrlc.store(true, Ordering::Relaxed);
    })
    .expect("failed to set Ctrl-C handler");

    let format = match cli.format {
        CliFormat::Code => sender::Format::Code,
        CliFormat::Embed => sender::Format::Embed,
    };

    let (batch_tx, batch_rx) = std::sync::mpsc::sync_channel::<String>(50);

    let sender_shutdown = shutdown.clone();
    let webhook = cli.webhook.clone();
    let tag = cli.tag.clone();
    let username = cli.username.clone();
    let dry_run = cli.dry_run;
    let quiet = cli.quiet;
    let max_messages = cli.max_messages;
    let max_bytes = cli.max_bytes;

    let sender_handle = std::thread::spawn(move || {
        sender_thread(
            batch_rx,
            &webhook,
            &tag,
            format,
            username.as_deref(),
            dry_run,
            quiet,
            sender_shutdown,
            max_messages,
            max_bytes,
        );
    });

    let strip_ansi = !cli.no_strip_ansi;
    let mut batch = batcher::BatchBuffer::new(cli.max_lines, cli.max_bytes);
    let window = Duration::from_millis(cli.window_ms);
    let mut last_flush = Instant::now();

    if let Some(ref follow_path) = cli.follow {
        let (line_tx, line_rx) = std::sync::mpsc::channel();
        let tail_shutdown = shutdown.clone();
        let path = follow_path.clone();
        std::thread::spawn(move || {
            reader::tail_file(&path, line_tx, strip_ansi, tail_shutdown);
        });

        while !shutdown.load(Ordering::Relaxed) {
            match line_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    batch.push_line(&line);
                    if batch.should_flush() {
                        let content = batch.drain();
                        if batch_tx.send(content).is_err() {
                            break;
                        }
                        last_flush = Instant::now();
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if last_flush.elapsed() >= window && !batch.is_empty() {
                        let content = batch.drain();
                        if batch_tx.send(content).is_err() {
                            break;
                        }
                        last_flush = Instant::now();
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    } else {
        let stdin = std::io::stdin();
        let line_reader = reader::LineReader::with_ansi_strip(stdin.lock(), strip_ansi);

        if cli.ndjson {
            let ndjson_iter = ndjson::NdjsonFilter::new(line_reader, cli.show_tool_calls);
            for line in ndjson_iter {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                batch.push_line(&line);
                if batch.should_flush() || last_flush.elapsed() >= window {
                    let content = batch.drain();
                    if batch_tx.send(content).is_err() {
                        break;
                    }
                    last_flush = Instant::now();
                }
            }
        } else {
            for line in line_reader {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                batch.push_line(&line);
                if batch.should_flush() || last_flush.elapsed() >= window {
                    let content = batch.drain();
                    if batch_tx.send(content).is_err() {
                        break;
                    }
                    last_flush = Instant::now();
                }
            }
        }
    }

    if !batch.is_empty() {
        let content = batch.drain();
        let _ = batch_tx.send(content);
    }

    drop(batch_tx);

    let _ = sender_handle.join();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_requires_webhook_url() {
        let result = Cli::try_parse_from(["discord-pipe"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_parses_webhook_flag() {
        let cli = Cli::try_parse_from([
            "discord-pipe",
            "--webhook",
            "https://discord.com/api/webhooks/123/abc",
        ])
        .unwrap();
        assert_eq!(cli.webhook, "https://discord.com/api/webhooks/123/abc");
    }

    #[test]
    fn cli_parses_all_options() {
        let cli = Cli::try_parse_from([
            "discord-pipe",
            "--webhook",
            "https://discord.com/api/webhooks/123/abc",
            "--tag",
            "my-build",
            "--window-ms",
            "3000",
            "--max-lines",
            "100",
            "--max-bytes",
            "1500",
            "--max-messages",
            "5",
            "--format",
            "embed",
            "--username",
            "BuildBot",
            "--no-strip-ansi",
            "--ndjson",
            "--show-tool-calls",
            "--dry-run",
            "--quiet",
        ])
        .unwrap();
        assert_eq!(cli.tag, "my-build");
        assert_eq!(cli.window_ms, 3000);
        assert_eq!(cli.max_lines, 100);
        assert_eq!(cli.max_bytes, 1500);
        assert_eq!(cli.max_messages, 5);
        assert_eq!(cli.format, CliFormat::Embed);
        assert_eq!(cli.username.as_deref(), Some("BuildBot"));
        assert!(cli.no_strip_ansi);
        assert!(cli.ndjson);
        assert!(cli.show_tool_calls);
        assert!(cli.dry_run);
        assert!(cli.quiet);
    }

    #[test]
    fn cli_parses_follow_flag() {
        let cli = Cli::try_parse_from([
            "discord-pipe",
            "--webhook",
            "https://discord.com/api/webhooks/123/abc",
            "--follow",
            "/var/log/app.log",
        ])
        .unwrap();
        assert_eq!(
            cli.follow.as_deref(),
            Some(std::path::Path::new("/var/log/app.log"))
        );
    }

    #[test]
    fn cli_defaults() {
        let cli = Cli::try_parse_from([
            "discord-pipe",
            "--webhook",
            "https://discord.com/api/webhooks/123/abc",
        ])
        .unwrap();
        assert_eq!(cli.tag, "discord-pipe");
        assert_eq!(cli.window_ms, 2000);
        assert_eq!(cli.max_lines, 50);
        assert_eq!(cli.max_bytes, 1800);
        assert_eq!(cli.max_messages, 3);
        assert_eq!(cli.format, CliFormat::Code);
        assert!(!cli.no_strip_ansi);
        assert!(!cli.ndjson);
        assert!(!cli.show_tool_calls);
        assert!(!cli.dry_run);
        assert!(!cli.quiet);
    }
}
