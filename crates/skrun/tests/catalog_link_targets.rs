//! The link targets of a skill body read back as the whole path.
//!
//! The tests run the built `skrun` binary as a subprocess, so they cover the
//! real command behavior: a link that writes its path between angle brackets
//! and a link whose path holds parentheses.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn a_link_target_reads_back_as_the_whole_path() {
    let dir = temp_dir("catalog-link-targets");
    write(
        &dir.join("demo/SKILL.md"),
        concat!(
            "---\n",
            "name: demo\n",
            "description: A demo skill.\n",
            "---\n",
            "\n",
            "# Demo\n",
            "\n",
            "Read the [guide](<docs/guide.md>) and the [v2 guide](docs/guide(v2).md).\n",
        ),
    );
    write(&dir.join("demo/docs/guide.md"), "# Guide\n");
    write(&dir.join("demo/docs/guide(v2).md"), "# Guide v2\n");
    let path = dir.to_str().unwrap();

    // The index reports both links with their whole path.
    let output = run(&["catalog", "index", path]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let index: serde_json::Value = serde_json::from_str(&stdout).expect("index json");
    let links = index["skills"][0]["links"].as_array().expect("links");
    assert_eq!(links.len(), 2, "{stdout}");
    let targets: Vec<String> = links
        .iter()
        .map(|link| link["target"].as_str().expect("target").to_string())
        .collect();
    assert!(
        targets.contains(&"demo/docs/guide.md".to_string()),
        "{stdout}"
    );
    assert!(
        targets.contains(&"demo/docs/guide(v2).md".to_string()),
        "{stdout}"
    );

    // Both links point at a file that exists.
    for link in links {
        assert_eq!(link["exists"], serde_json::Value::Bool(true), "{stdout}");
    }

    // The check finds no broken link.
    let output = run(&["catalog", "check", path]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("broken link"), "{stdout}");
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
    // The clock of the system is coarse, so two tests of one process can
    // read the same time. The counter keeps the name of every folder
    // unique, so two tests never share a folder.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "skrun-link-targets-{name}-{}-{now}-{count}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
