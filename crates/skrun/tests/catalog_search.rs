//! The catalog search command finds the skills of a directory by keyword.
//!
//! The test runs the built `skrun` binary as a subprocess, so it covers the
//! real command behavior: the exit status of a search with a match, of a
//! search without a match and of a search without a query.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn a_search_reports_its_exit_status() {
    let dir = fixture("catalog-search");
    let path = dir.to_str().unwrap();

    // A query that matches a skill ends the command with status 0.
    let output = run(&["catalog", "search", path, "模型"]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ai-tools"), "{stdout}");

    // A query that matches no skill ends the command with status 1.
    let output = run(&["catalog", "search", path, "nothing-matches"]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no skills match"), "{stdout}");

    // A search without a query is a usage error.
    let output = run(&["catalog", "search", path]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing query"), "{stderr}");
}

/// Run the built binary with the given arguments.
fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .output()
        .expect("run the skrun binary")
}

/// A skills directory with a Chinese skill, a skill with a script and a flat
/// skill.
fn fixture(name: &str) -> PathBuf {
    let dir = temp_dir(name);
    write(
        &dir.join("ai-tools/SKILL.md"),
        concat!(
            "---\n",
            "name: AI Tools\n",
            "description: 大模型工具箱，帮助选择模型。\n",
            "---\n",
            "\n",
            "# AI Tools\n",
        ),
    );
    write(
        &dir.join("secrets/SKILL.md"),
        "---\nname: Secrets\ndescription: Keeps credentials safe.\n---\n\n# Secrets\n",
    );
    write(&dir.join("secrets/scripts/tool.py"), "print(\"hello\")\n");
    write(
        &dir.join("deploy-guide.md"),
        "# Deploy guide\n\nCopy files with rsync to the host.\n",
    );
    write(
        &dir.join("SKILL_REGISTRY.md"),
        concat!(
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| AI Tools | When you need a model | [ai-tools](ai-tools/SKILL.md) |\n",
            "| Secrets | When you handle keys | [secrets](secrets/SKILL.md) |\n",
            "| Deploy Guide | When you deploy | [deploy-guide](deploy-guide.md) |\n",
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
        "skrun-catalog-search-{name}-{}-{now}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
