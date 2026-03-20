use clap::Parser;

#[allow(dead_code)]
mod ansi;
#[allow(dead_code)]
mod batcher;
#[allow(dead_code)]
mod format;
#[allow(dead_code)]
mod reader;
#[allow(dead_code)]
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
    pub dry_run: bool,

    #[arg(long)]
    pub quiet: bool,
}

fn main() {
    dotenvy::dotenv().ok();
    let _cli = Cli::parse();
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
        assert!(!cli.dry_run);
        assert!(!cli.quiet);
    }
}
