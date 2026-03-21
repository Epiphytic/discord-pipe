use std::process::Command;
use std::time::Duration;

#[test]
fn dry_run_batches_stdin_to_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args([
            "--webhook",
            "https://discord.com/api/webhooks/fake/fake",
            "--dry-run",
            "--tag",
            "test",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"line1\nline2\nline3\n")
                .unwrap();
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("line1"), "stdout was: {stdout}");
    assert!(stdout.contains("line2"), "stdout was: {stdout}");
    assert!(stdout.contains("line3"), "stdout was: {stdout}");
    assert!(stdout.contains("```"), "stdout was: {stdout}");
    assert!(stdout.contains("[test]"), "stdout was: {stdout}");
}

#[test]
fn dry_run_strips_ansi_by_default() {
    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args([
            "--webhook",
            "https://discord.com/api/webhooks/fake/fake",
            "--dry-run",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"\x1b[31mred\x1b[0m\n")
                .unwrap();
            child.wait_with_output()
        })
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("red"), "stdout was: {stdout}");
    assert!(!stdout.contains("\x1b["), "stdout was: {stdout}");
}

#[test]
fn exits_with_error_on_missing_webhook() {
    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .env_remove("DISCORD_WEBHOOK_URL")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();

    assert!(!output.status.success());
}

#[test]
fn dry_run_tails_file() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("test.log");
    std::fs::write(&log_path, "").unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args([
            "--webhook",
            "https://discord.com/api/webhooks/fake/fake",
            "--dry-run",
            "--follow",
            log_path.to_str().unwrap(),
            "--window-ms",
            "500",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    std::thread::sleep(Duration::from_millis(500));

    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .unwrap();
    writeln!(f, "tailed line 1").unwrap();
    writeln!(f, "tailed line 2").unwrap();
    f.flush().unwrap();

    std::thread::sleep(Duration::from_millis(1500));

    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("tailed line 1"), "stdout was: {stdout}");
    assert!(stdout.contains("tailed line 2"), "stdout was: {stdout}");
}

#[test]
fn ndjson_mode_extracts_assistant_text() {
    let input = [
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello from jcode"}]}}"#,
        r#"{"type":"tool_use","name":"file_read","input":{"file_path":"src/main.rs"}}"#,
        r#"{"type":"tool_result","content":"file contents here"}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Build succeeded"}]}}"#,
        r#"{"type":"token_usage","input":100,"output":50}"#,
    ]
    .join("\n")
        + "\n";

    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args([
            "--webhook",
            "https://discord.com/api/webhooks/fake/fake",
            "--dry-run",
            "--ndjson",
            "--tag",
            "ndjson-test",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Hello from jcode"),
        "should contain assistant text: {stdout}"
    );
    assert!(
        stdout.contains("Build succeeded"),
        "should contain second assistant text: {stdout}"
    );
    assert!(
        !stdout.contains("file_read"),
        "should filter tool_use: {stdout}"
    );
    assert!(
        !stdout.contains("tool_result"),
        "should filter tool_result: {stdout}"
    );
    assert!(
        !stdout.contains("token_usage"),
        "should filter token usage: {stdout}"
    );
}

#[test]
fn ndjson_mode_shows_tool_calls_with_wrench_emoji() {
    let input = [
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Starting"}]}}"#,
        r#"{"type":"tool_use","name":"grep","input":{"pattern":"TODO","path":"src/"}}"#,
        r#"{"type":"tool_result","content":"found 3 matches"}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}"#,
    ]
    .join("\n")
        + "\n";

    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args([
            "--webhook",
            "https://discord.com/api/webhooks/fake/fake",
            "--dry-run",
            "--ndjson",
            "--show-tool-calls",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Starting"),
        "should contain assistant text: {stdout}"
    );
    assert!(stdout.contains("grep"), "should show tool name: {stdout}");
    assert!(
        stdout.contains("\u{1f527}"),
        "should have wrench emoji: {stdout}"
    );
    assert!(stdout.contains("TODO"), "should show tool args: {stdout}");
    assert!(
        stdout.contains("Done"),
        "should contain second assistant text: {stdout}"
    );
    assert!(
        !stdout.contains("tool_result"),
        "should still filter tool_result: {stdout}"
    );
}

#[test]
fn ndjson_mode_handles_plain_text_lines() {
    let input = "Just plain text\nAnother line\n";

    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args([
            "--webhook",
            "https://discord.com/api/webhooks/fake/fake",
            "--dry-run",
            "--ndjson",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Just plain text"),
        "should pass through plain text: {stdout}"
    );
    assert!(
        stdout.contains("Another line"),
        "should pass through all plain text: {stdout}"
    );
}

#[test]
fn ndjson_backward_compat_text_type() {
    let input = [
        r#"{"type":"text","content":"Legacy text format"}"#,
        r#"{"type":"text","text":"Also legacy"}"#,
    ]
    .join("\n")
        + "\n";

    let output = Command::new(env!("CARGO_BIN_EXE_discord-pipe"))
        .args([
            "--webhook",
            "https://discord.com/api/webhooks/fake/fake",
            "--dry-run",
            "--ndjson",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Legacy text format"),
        "should handle content field: {stdout}"
    );
    assert!(
        stdout.contains("Also legacy"),
        "should handle text field: {stdout}"
    );
}
