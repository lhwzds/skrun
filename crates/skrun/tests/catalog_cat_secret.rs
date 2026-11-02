//! The catalog cat command never prints a secret of a stored file.
//!
//! The tests run the built `skrun` binary as a subprocess, so they cover the
//! real command output: stdout and stderr together, exactly as a user sees
//! them. Every fixture is written by the test itself and is only ever copied,
//! hashed and scanned, never run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// A key shaped value that must never reach the command output.
const SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

/// A password of a line that must never reach the command output.
const PASSWORD: &str = "hunter2xyz";

/// The material of a key of a private key that must never reach the command
/// output.
const KEY_MATERIAL: &str = "MIIEowIBAAKCAQEAx1234567890";

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn the_cat_command_never_prints_a_secret_of_a_stored_file() {
    let source = skill_fixture("cat-secret");
    let store = temp_dir("cat-secret-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "docs/notes.md",
        "--store",
        &path(&store),
    ]);

    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output);

    // The lines that hold no secret are printed as they are, so the text of
    // the file stays readable.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Notes"), "{stdout}");
    assert!(stdout.contains("An ordinary line."), "{stdout}");
    assert!(stdout.contains("Another ordinary line."), "{stdout}");

    // The lines that hold a secret are hidden, so the secret is never part
    // of the output.
    assert!(stdout.contains("<redacted>"), "{stdout}");
}

#[test]
fn the_cat_command_never_prints_a_secret_with_require_safe() {
    let source = skill_fixture("cat-secret-safe");
    let store = temp_dir("cat-secret-safe-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "docs/notes.md",
        "--store",
        &path(&store),
        "--require-safe",
    ]);

    // The version holds no high finding, so it is safe to read and the file
    // is printed. The secret of the file is hidden all the same.
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("An ordinary line."), "{stdout}");
    assert!(stdout.contains("<redacted>"), "{stdout}");
}

#[test]
fn the_cat_command_never_prints_a_secret_of_the_skill_itself() {
    let source = skill_fixture("cat-secret-skill");
    // The file of the skill itself holds a secret, so the file that the
    // command shows when no file is named holds one as well.
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n\npassword: hunter2xyz\n",
    );
    let store = temp_dir("cat-secret-skill-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The file of the skill itself is printed when no file is named.
    let output = run(&["catalog", "cat", "demo", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Demo"), "{stdout}");
    assert!(stdout.contains("<redacted>"), "{stdout}");
}

#[test]
fn the_cat_command_never_prints_a_secret_shaped_file_name() {
    let source = skill_fixture("cat-secret-name");
    let store = temp_dir("cat-secret-name-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&["catalog", "cat", "demo", "--list", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SKILL.md"), "{stdout}");
    assert!(stdout.contains("docs/notes.md"), "{stdout}");
}

#[test]
fn the_cat_command_never_prints_the_body_of_a_private_key() {
    let source = skill_fixture("cat-secret-key");
    // The last line of the key of the private key has no line ending, so
    // it is the last line of the file.
    write(
        &source.join("docs/key.md"),
        &format!("-----BEGIN RSA PRIVATE KEY-----\n{KEY_MATERIAL}"),
    );
    let store = temp_dir("cat-secret-key-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "docs/key.md",
        "--store",
        &path(&store),
    ]);

    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output);

    // The material of the key is hidden, so it never reaches the output.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("<redacted>"), "{stdout}");
}

#[test]
fn the_cat_command_prints_the_text_behind_a_private_key_of_one_line() {
    let source = skill_fixture("cat-secret-key-one-line");
    // The start and the end of the key of the private key are in one line,
    // so the key ends in that line and the text behind it is no secret of
    // the key.
    write(
        &source.join("docs/key.md"),
        &format!(
            "-----BEGIN RSA PRIVATE KEY----- {KEY_MATERIAL} -----END RSA PRIVATE KEY-----\nAn ordinary line.\n"
        ),
    );
    let store = temp_dir("cat-secret-key-one-line-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "docs/key.md",
        "--store",
        &path(&store),
    ]);

    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output);

    // The text behind the key is printed as it is.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("An ordinary line."), "{stdout}");
}

#[test]
fn the_cat_command_redacts_every_line_of_a_file_with_carriage_returns() {
    let source = skill_fixture("cat-secret-cr");
    // The lines of the file are separated by carriage returns only, so the
    // file is read line by line all the same.
    write(
        &source.join("docs/notes.md"),
        &format!("An ordinary line.\rpassword: {PASSWORD}\rAnother ordinary line.\r"),
    );
    let store = temp_dir("cat-secret-cr-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "docs/notes.md",
        "--store",
        &path(&store),
    ]);

    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output);

    // The lines that hold no secret are printed as they are, and the line
    // that holds a secret is hidden.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("An ordinary line."), "{stdout}");
    assert!(stdout.contains("Another ordinary line."), "{stdout}");
    assert!(stdout.contains("<redacted>"), "{stdout}");
}

/// Assert that neither stream of the command holds a secret.
fn assert_clean(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for secret in [SECRET, PASSWORD, KEY_MATERIAL] {
        assert!(!stdout.contains(secret), "stdout: {stdout}");
        assert!(!stderr.contains(secret), "stderr: {stderr}");
    }
}

/// A skill directory with a file that holds a secret.
///
/// The file of the skill holds a key and a password, and it holds lines that
/// hold no secret as well, so the command has to hide the secrets and to
/// print the other lines as they are.
fn skill_fixture(name: &str) -> PathBuf {
    let dir = temp_dir(name).join("demo");
    write(
        &dir.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    write(
        &dir.join("docs/notes.md"),
        &format!(
            "# Notes\n\nAn ordinary line.\n\naws_key = {SECRET}\npassword: {PASSWORD}\n\nAnother ordinary line.\n"
        ),
    );
    write(
        &dir.join(format!("docs/{SECRET}.md")),
        "# A file with a key in its name.\n",
    );
    dir
}

/// The text of a path.
fn path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Run the binary with the given arguments.
fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .output()
        .expect("run the skrun binary")
}

/// Write a file, creating the parent folders.
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
    // The clock of the system is coarse, so two tests of one process can
    // read the same time. The counter keeps the name of every folder
    // unique, so two tests never share a folder.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "skrun-cat-secret-test-{name}-{}-{now}-{count}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
