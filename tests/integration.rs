use std::process::Command;

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
