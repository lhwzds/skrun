//! The list values of a frontmatter keep the commas of their quoted items.
//!
//! The test runs the built `skrun` binary as a subprocess, so it covers the
//! real index output: a quoted tag that holds a comma stays one tag and
//! every item of a list loses its quotes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn a_quoted_item_keeps_its_comma() {
    let dir = fixture("catalog-list-values");
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "index", path]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let index: serde_json::Value = serde_json::from_str(&stdout).expect("the index is json");
    let skill = &index["skills"][0];
    assert_eq!(
        skill["tags"],
        serde_json::json!(["one, two", "three", "four"]),
        "{stdout}"
    );
    assert_eq!(skill["requires"], serde_json::json!(["a", "b"]), "{stdout}");
}

#[test]
fn a_list_that_is_written_without_brackets_keeps_its_quoted_items() {
    let dir = temp_dir("catalog-list-values-plain");
    write(
        &dir.join("demo/SKILL.md"),
        concat!(
            "---\n",
            "name: Demo\n",
            "description: A demo skill.\n",
            "tags: \"one, two\", 'three', four\n",
            "---\n",
            "\n",
            "# Demo\n",
        ),
    );
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "index", path]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let index: serde_json::Value = serde_json::from_str(&stdout).expect("the index is json");
    assert_eq!(
        index["skills"][0]["tags"],
        serde_json::json!(["one, two", "three", "four"]),
        "{stdout}"
    );
}

/// Run the built binary with the given arguments.
fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .output()
        .expect("run the skrun binary")
}

/// A skills directory with a skill whose tags hold a quoted comma.
fn fixture(name: &str) -> PathBuf {
    let dir = temp_dir(name);
    write(
        &dir.join("demo/SKILL.md"),
        concat!(
            "---\n",
            "name: Demo\n",
            "description: A demo skill.\n",
            "tags: [\"one, two\", 'three', four]\n",
            "requires: [a, b]\n",
            "---\n",
            "\n",
            "# Demo\n",
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
        "skrun-catalog-list-{name}-{}-{now}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
