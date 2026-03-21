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
fn ndjson_mode_extracts_text_events() {
    let input = [
        r#"{"type":"text","content":"Hello from jcode"}"#,
        r#"{"type":"tool_call","name":"read","content":"reading file"}"#,
        r#"{"type":"text","content":"Build succeeded"}"#,
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
        "should contain text event: {stdout}"
    );
    assert!(
        stdout.contains("Build succeeded"),
        "should contain second text event: {stdout}"
    );
    assert!(
        !stdout.contains("tool_call"),
        "should filter tool calls: {stdout}"
    );
    assert!(
        !stdout.contains("token_usage"),
        "should filter token usage: {stdout}"
    );
}

#[test]
fn ndjson_mode_shows_tool_calls_when_flag_set() {
    let input = [
        r#"{"type":"text","content":"Starting"}"#,
        r#"{"type":"tool_call","name":"grep"}"#,
        r#"{"type":"text","content":"Done"}"#,
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
    assert!(stdout.contains("Starting"), "stdout was: {stdout}");
    assert!(
        stdout.contains("[tool: grep]"),
        "should show tool call: {stdout}"
    );
    assert!(stdout.contains("Done"), "stdout was: {stdout}");
}

#[test]
fn ndjson_mode_handles_mixed_json_and_plain_text() {
    let input = "plain text line\n{\"type\":\"text\",\"content\":\"json text\"}\nnot json either\n";

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
        stdout.contains("plain text line"),
        "should pass plain text: {stdout}"
    );
    assert!(
        stdout.contains("json text"),
        "should extract json text: {stdout}"
    );
    assert!(
        stdout.contains("not json either"),
        "should pass non-json: {stdout}"
    );
}
