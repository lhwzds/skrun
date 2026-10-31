//! The catalog commands never print a value that looks like a secret.
//!
//! The tests run the built `skrun` binary as a subprocess, so they cover the
//! real command output: stdout and stderr together, exactly as a user sees
//! them. The last test covers the registry escaping: a title with pipes and
//! backslashes has to survive a write and a dry run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// A key shaped value that must never reach the command output.
const SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

/// A synthetic token of every shape the catalog has to hide.
///
/// The tokens are made up, they hold no real key; they only have the shape
/// of one, so the catalog has to treat them as a secret.
const TOKENS: [&str; 5] = [
    "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789",
    "sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345",
    "github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz",
    "glpat-abcdefghijklmnopqrstuvwxyz0123456789",
    "AIzaSyA1234567890abcdefghijklmnopqrstuv",
];

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn catalog_commands_never_print_a_secret() {
    let dir = secret_fixture("catalog-secret-output");
    let path = dir.to_str().unwrap();

    // The check reports the findings of the directory.
    let output = run(&["catalog", "check", path]);
    assert_ne!(output.status.code(), Some(0));
    assert_clean(&output);

    // The registry dry run reports the drift of the table.
    let output = run(&["catalog", "registry", path]);
    assert_ne!(output.status.code(), Some(0));
    assert_clean(&output);

    // The registry write repairs the table and reports what it did.
    let output = run(&["catalog", "registry", path, "--write"]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output);

    // An unknown option is echoed back, but never as a secret.
    let output = run(&["catalog", "check", path, &format!("--{SECRET}")]);
    assert_ne!(output.status.code(), Some(0));
    assert_clean(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown option `<redacted>`"), "{stderr}");
}

#[test]
fn the_registry_write_keeps_the_secret_out_of_the_report() {
    let dir = secret_fixture("catalog-secret-write");
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "registry", path, "--write"]);

    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("<redacted>"), "{stdout}");
}

#[test]
fn a_title_with_pipes_and_backslashes_round_trips() {
    let dir = escaped_title_fixture("catalog-escaped-title");
    let path = dir.to_str().unwrap();

    // The write repairs the table and reports what it did.
    let output = run(&["catalog", "registry", path, "--write"]);
    assert_eq!(output.status.code(), Some(0));

    let registry = fs::read_to_string(dir.join("SKILL_REGISTRY.md")).unwrap();
    assert!(
        registry.contains(
            r"| Pipe \\\| Skill \| Second | A summary with \\\| and \|. | [demo](demo/SKILL.md) |"
        ),
        "{registry}"
    );

    // The written row reads back as the same three cells, so the table is in
    // sync and the dry run succeeds.
    let output = run(&["catalog", "registry", path]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: in sync"), "{stdout}");
}

#[test]
fn the_new_secret_shapes_never_reach_the_output() {
    let dir = secret_shapes_fixture("catalog-secret-shapes");
    let path = dir.to_str().unwrap();

    // The check reports every token by its path and its line.
    let output = run(&["catalog", "check", path]);
    assert_ne!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in 2..=6 {
        assert!(
            stdout.contains(&format!("demo/notes.txt:{line}")),
            "stdout: {stdout}"
        );
    }
    assert_clean(&output);
    assert_clean_tokens(&output);

    // The registry commands never print a token either.
    let output = run(&["catalog", "registry", path]);
    assert_ne!(output.status.code(), Some(0));
    assert_clean_tokens(&output);

    let output = run(&["catalog", "registry", path, "--write"]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean_tokens(&output);

    // An unknown option is echoed back, but never as a secret.
    for token in TOKENS {
        let output = run(&["catalog", "check", path, &format!("--{token}")]);
        assert_ne!(output.status.code(), Some(0));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("unknown option `<redacted>`"), "{stderr}");
        assert!(!stderr.contains(token), "stderr: {stderr}");
    }
}

/// Run the built binary with the given arguments.
fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .output()
        .expect("run the skrun binary")
}

/// Assert that neither stream of the command holds the planted secret.
fn assert_clean(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(SECRET), "stdout: {stdout}");
    assert!(!stderr.contains(SECRET), "stderr: {stderr}");
}

/// Assert that neither stream of the command holds a token of any shape.
fn assert_clean_tokens(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for token in TOKENS {
        assert!(!stdout.contains(token), "stdout: {stdout}");
        assert!(!stderr.contains(token), "stderr: {stderr}");
    }
}

/// A skills directory whose frontmatter name, link target, file name and
/// registry row all hold a key shaped value.
fn secret_fixture(name: &str) -> PathBuf {
    let dir = temp_dir(name);
    write(
        &dir.join("demo/SKILL.md"),
        concat!(
            "---\n",
            "name: Demo AKIAIOSFODNN7EXAMPLE\n",
            "description: A demo skill.\n",
            "---\n",
            "\n",
            "# Demo\n",
            "\n",
            "Read the [missing page](docs/AKIAIOSFODNN7EXAMPLE.md).\n",
        ),
    );
    write(
        &dir.join(format!("{SECRET}.md")),
        "the key is AKIAIOSFODNN7EXAMPLE\n",
    );
    write(
        &dir.join("SKILL_REGISTRY.md"),
        concat!(
            "# Skills\n",
            "\n",
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Broken | When breaking | AKIAIOSFODNN7EXAMPLE |\n",
        ),
    );
    dir
}

/// A skills directory that holds a token of every shape.
///
/// The tokens live in a note of the demo skill and in a row of the registry,
/// so both the check and the registry command have to hide them.
fn secret_shapes_fixture(name: &str) -> PathBuf {
    let dir = temp_dir(name);
    write(
        &dir.join("demo/SKILL.md"),
        concat!(
            "---\n",
            "name: Demo\n",
            "description: A demo skill.\n",
            "---\n",
            "\n",
            "# Demo\n",
        ),
    );
    write(
        &dir.join("demo/notes.txt"),
        concat!(
            "keys:\n",
            "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789\n",
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345\n",
            "github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz\n",
            "glpat-abcdefghijklmnopqrstuvwxyz0123456789\n",
            "AIzaSyA1234567890abcdefghijklmnopqrstuv\n",
        ),
    );
    write(
        &dir.join("SKILL_REGISTRY.md"),
        concat!(
            "# Skills\n",
            "\n",
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Leaked | When leaking | sk-proj-abcdefghijklmnopqrstuvwxyz0123456789 |\n",
        ),
    );
    dir
}

/// A skills directory whose title and summary hold pipes and backslashes.
fn escaped_title_fixture(name: &str) -> PathBuf {
    let dir = temp_dir(name);
    write(
        &dir.join("demo/SKILL.md"),
        concat!(
            "---\n",
            "name: Pipe \\| Skill | Second\n",
            "description: A summary with \\| and |.\n",
            "---\n",
            "\n",
            "# Demo\n",
        ),
    );
    write(
        &dir.join("SKILL_REGISTRY.md"),
        concat!(
            "# Skills\n",
            "\n",
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
        ),
    );
    dir
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
        "skrun-secret-output-{name}-{}-{now}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
