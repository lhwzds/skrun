//! The escapes of the strings of a python script.
//!
//! The tests write a fixture with a python script that holds the escapes of
//! a string and add the fixture to a store. A fixture is only ever copied,
//! hashed and read: nothing of a fixture is ever run.

use skrun::store::{self, AddOptions, StoredVersion};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A script whose strings hold an escape of a wide character.
const WIDE_ESCAPES: &str = concat!(
    "text = \"\\\u{e9}\"\n",
    "other = '\\\u{4e2d}'\n",
    "third = \"\\\\\"\n",
    "import subprocess\n",
);

/// A script whose string of three quotes holds an escaped quote.
const ESCAPED_QUOTE: &str = concat!("x = \"\"\"a \\\"\"\" b\"\"\"\n", "import subprocess\n",);

#[test]
fn a_string_with_an_escape_of_a_wide_character_does_not_hide_an_import() {
    let version = add_skill("wide-escapes", WIDE_ESCAPES);

    assert_eq!(version.tier, "L2");
    assert!(
        version.scripts[0]
            .imports
            .contains(&"subprocess".to_string()),
        "{:?}",
        version.scripts
    );
    assert!(
        version
            .tier_reasons
            .iter()
            .any(|reason| reason.contains("risky import: subprocess")),
        "{:?}",
        version.tier_reasons
    );
}

#[test]
fn an_escaped_quote_does_not_close_a_string_of_three_quotes() {
    let version = add_skill("escaped-quote", ESCAPED_QUOTE);

    assert_eq!(version.tier, "L2");
    assert!(
        version.scripts[0]
            .imports
            .contains(&"subprocess".to_string()),
        "{:?}",
        version.scripts
    );
    assert!(
        version
            .tier_reasons
            .iter()
            .any(|reason| reason.contains("risky import: subprocess")),
        "{:?}",
        version.tier_reasons
    );
}

/// Add a skill with a python script to a store and return the version.
fn add_skill(name: &str, script: &str) -> StoredVersion {
    let source = skill_fixture(name, script);
    let store_path = temp_dir(&format!("{name}-store"));
    store::add_skill(&AddOptions {
        source: source.to_path_buf(),
        store: Some(store_path.to_path_buf()),
        id: None,
        from: None,
    })
    .expect("add the skill to the store")
    .version
}

/// A skill directory with a python script.
///
/// The skill lives in a folder named `demo`, because the id of a skill comes
/// from the name of its source.
fn skill_fixture(name: &str, script: &str) -> PathBuf {
    let dir = temp_dir(name).join("demo");
    write(
        &dir.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    write(&dir.join("x/run.py"), script);
    dir
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
        "skrun-python-escape-test-{name}-{}-{now}-{count}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
