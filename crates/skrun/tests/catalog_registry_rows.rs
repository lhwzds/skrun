//! The registry table survives the markdown around it.
//!
//! The tests run the built `skrun` binary as a subprocess, so they cover the
//! real command behavior: a fenced example that holds the registry header, a
//! line of inline code that looks like a fence and a skill folder whose name
//! holds parentheses.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn a_fenced_example_in_a_list_item_is_left_alone() {
    let dir = temp_dir("catalog-list-fence");
    write(
        &dir.join("demo/SKILL.md"),
        "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The example inside the list item holds the exact registry header; it is
    // a fenced example and not the registry table.
    let fence = concat!(
        "- ```markdown\n",
        "  | Skill | When to read | Entrypoint |\n",
        "  | --- | --- | --- |\n",
        "  | Fake | When faking | [fake](fake/SKILL.md) |\n",
        "  ```\n",
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

#[test]
fn a_closing_fence_in_a_block_quote_is_found() {
    let dir = temp_dir("catalog-quote-fence");
    write(
        &dir.join("demo/SKILL.md"),
        "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The closing fence sits behind the block quote marker and three spaces;
    // it closes the quoted example, so the table below it is the registry
    // table.
    write(
        &dir.join("SKILL_REGISTRY.md"),
        concat!(
            "# Skills\n",
            "\n",
            "> ```markdown\n",
            "> | Skill | When to read | Entrypoint |\n",
            "> | --- | --- | --- |\n",
            "> | Fake | When faking | [fake](fake/SKILL.md) |\n",
            ">    ```\n",
            "\n",
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Demo | When demoing | [demo](demo/SKILL.md) |\n",
        ),
    );
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "registry", path]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: in sync"), "{stdout}");
}

#[test]
fn an_inline_code_line_is_not_a_fence() {
    let dir = temp_dir("catalog-inline-code");
    write(
        &dir.join("demo/SKILL.md"),
        "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The info string of the line holds a backtick, so the line is inline
    // code and not a fence; the table below it is the registry table.
    write(
        &dir.join("SKILL_REGISTRY.md"),
        concat!(
            "# Skills\n",
            "\n",
            "```code``` here\n",
            "\n",
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Demo | When demoing | [demo](demo/SKILL.md) |\n",
        ),
    );
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "registry", path]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: in sync"), "{stdout}");
}

#[test]
fn a_skill_folder_with_parentheses_round_trips() {
    let dir = temp_dir("catalog-parentheses");
    write(
        &dir.join("demo(v2)/SKILL.md"),
        "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    write(
        &dir.join("SKILL_REGISTRY.md"),
        "| Skill | When to read | Entrypoint |\n| --- | --- | --- |\n",
    );
    let path = dir.to_str().unwrap();

    // The write appends the row of the skill.
    let output = run(&["catalog", "registry", path, "--write"]);
    assert_eq!(output.status.code(), Some(0));

    let after = fs::read_to_string(dir.join("SKILL_REGISTRY.md")).unwrap();
    assert!(after.contains("[demo(v2)](<demo(v2)/SKILL.md>)"), "{after}");

    // The written row reads back as the same entrypoint, so the dry run
    // finds the table in sync.
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
    let root = std::env::temp_dir().join(format!(
        "skrun-registry-rows-{name}-{}-{now}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
