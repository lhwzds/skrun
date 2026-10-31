//! The rule of a key and a value that hold a secret.
//!
//! The tests run the built `skrun` binary as a subprocess, so they cover the
//! real command behavior: a line that names no secret is no finding, and a
//! line that holds a secret is a finding that never prints the value.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

/// The lines that name no secret, so no line of them is a finding.
const NO_SECRET: [&str; 10] = [
    "max_tokens:4096",
    "query_tokens = ...",
    "token: Token = []",
    "subTest(token=x)",
    "\"x-api-key\": $key",
    "x-api-key: YOUR_KEY",
    "password: get_secret (arg)",
    "password: lookup = other",
    "password: ${TOKEN}suffix",
    "password: \"abcdef\" + get_secret()",
];

/// The lines that hold a secret, so every line of them is a finding.
const SECRET: [&str; 8] = [
    "password: hunter2xyz",
    "\"password\": \"hunter2xyz\"",
    "api_key = \"sk_live_...\"",
    "POSTGRES_PASSWORD: postgres123",
    "password: \"Hunter2xyz\"",
    "{password: hunter2xyz, retries: [1, 2]}",
    "{config: {password: hunter2xyz}, retries: [1, 2]}",
    "[{password: hunter2xyz}, {other: 1}]",
];

#[test]
fn a_line_that_names_no_secret_is_no_finding() {
    let dir = fixture("catalog-secret-rule-none", &NO_SECRET);
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "check", path]);

    // The check finds no secret, so it reports no error.
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("possible secret"), "stdout: {stdout}");
    assert!(!stderr.contains("possible secret"), "stderr: {stderr}");
    // No text of the lines is redacted, so no text of them is hidden.
    assert!(!stdout.contains("<redacted>"), "stdout: {stdout}");
    assert!(!stderr.contains("<redacted>"), "stderr: {stderr}");
}

#[test]
fn a_line_that_holds_a_secret_is_a_finding() {
    let dir = fixture("catalog-secret-rule-some", &SECRET);
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "check", path]);

    // The check finds every line of the file.
    assert_ne!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in 1..=SECRET.len() {
        assert!(
            stdout.contains(&format!("possible secret in `demo/notes.txt:{line}`")),
            "stdout: {stdout}"
        );
    }
    // The finding keeps the place of the secret, never its value.
    for value in ["hunter2xyz", "Hunter2xyz", "sk_live_", "postgres123"] {
        assert!(!stdout.contains(value), "stdout: {stdout}");
    }
}

/// A skills directory whose notes hold the given lines.
fn fixture(name: &str, lines: &[&str]) -> PathBuf {
    let dir = temp_dir(name);
    write(
        &dir.join("demo/SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    let mut notes = String::new();
    for line in lines {
        notes.push_str(line);
        notes.push('\n');
    }
    write(&dir.join("demo/notes.txt"), &notes);
    dir
}

/// Run the built binary with the given arguments.
fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .output()
        .expect("run the skrun binary")
}

/// Write a file, creating its parent directories.
fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

/// A fresh directory below the system temporary directory.
fn temp_dir(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "skrun-secret-rule-{name}-{}-{now}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
