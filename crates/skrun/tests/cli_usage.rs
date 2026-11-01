//! The usage text of the command line tells what every command takes.
//!
//! Every test runs the binary with `--help` and reads the text that the
//! binary prints. Nothing of a fixture is ever run.

use std::process::Command;

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn the_usage_text_shows_the_list_of_the_store() {
    // The list of the store takes the same options as the record of one
    // skill, so the usage text shows them as well.
    let output = Command::new(BINARY)
        .arg("--help")
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("skrun catalog inspect --list [--store <dir>] [--json]"),
        "stdout: {stdout}"
    );
}

#[test]
fn the_usage_text_shows_the_record_of_one_skill() {
    let output = Command::new(BINARY)
        .arg("--help")
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("skrun catalog inspect <id> [--store <dir>] [--json]"),
        "stdout: {stdout}"
    );
}

#[test]
fn the_usage_text_shows_the_path_of_a_version() {
    let output = Command::new(BINARY)
        .arg("--help")
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("skrun catalog path <id> [--store <dir>] [--version <hash>]"),
        "stdout: {stdout}"
    );
}

#[test]
fn the_usage_text_shows_the_file_of_a_version() {
    let output = Command::new(BINARY)
        .arg("--help")
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "skrun catalog cat <id> [<relative-file>] [--store <dir>] [--version <hash>] [--require-safe]"
        ),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("skrun catalog cat <id> --list [--store <dir>] [--version <hash>]"),
        "stdout: {stdout}"
    );
}
