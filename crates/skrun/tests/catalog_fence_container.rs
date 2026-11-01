//! A fence is only closed by a fence in the same container.
//!
//! The tests run the built `skrun` binary as a subprocess, so they cover the
//! real command behavior: a fenced example that holds the registry header and
//! a list item that looks like a closing fence.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn a_list_item_inside_a_top_level_fence_is_content() {
    let dir = temp_dir("catalog-top-level-fence");
    write(
        &dir.join("demo/SKILL.md"),
        "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The example holds the exact registry header and a fake row. The line
    // "- ```" looks like a closing fence, but it sits in a list item, so it
    // is content of the top level fence and not a closing fence.
    let fence = concat!(
        "```markdown\n",
        "- ```\n",
        "| Skill | When to read | Entrypoint |\n",
        "| --- | --- | --- |\n",
        "| Fake | When faking | [fake](fake/SKILL.md) |\n",
        "```\n",
    );
    let table = concat!(
        "| Skill | When to read | Entrypoint |\n",
        "| --- | --- | --- |\n",
        "| Ghost | When ghosting | [ghost](ghost/SKILL.md) |\n",
    );
    write(
        &dir.join("SKILL_REGISTRY.md"),
        &format!("# Skills\n\n{fence}\n{table}\nTrailing text.\n"),
    );
    let path = dir.to_str().unwrap();

    // The write repairs the real table.
    let output = run(&["catalog", "registry", path, "--write"]);
    assert_eq!(output.status.code(), Some(0));

    let after = fs::read_to_string(dir.join("SKILL_REGISTRY.md")).unwrap();
    // The fenced example keeps its bytes.
    assert!(after.contains(fence), "{after}");
    // The real table is reconciled.
    assert!(
        after.contains("| Demo | A demo skill. | [demo](demo/SKILL.md) |"),
        "{after}"
    );
    assert!(!after.contains("ghost/SKILL.md"), "{after}");
    assert!(after.ends_with("\nTrailing text.\n"), "{after}");

    // The dry run finds the same table and reports it as in sync.
    let output = run(&["catalog", "registry", path]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: in sync"), "{stdout}");
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
        "skrun-fence-container-{name}-{}-{now}-{count}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
