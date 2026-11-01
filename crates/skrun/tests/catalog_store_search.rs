//! The search of the store finds the skills of the store by keyword.
//!
//! The tests run the built `skrun` binary as a subprocess, so they cover
//! the real command behavior: the exit status of a search with a match, of a
//! search without a match and of a search of the default store. Every
//! fixture is only ever copied, hashed and scanned: nothing of a fixture is
//! ever run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

/// The word that every skill of the fixture holds.
const QUERY: &str = "widget";

#[test]
fn a_search_of_the_store_finds_every_stored_skill() {
    let store = store_fixture("store-search");

    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        QUERY,
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Every skill of the store is a result of the search.
    assert!(stdout.contains("guide"), "{stdout}");
    assert!(stdout.contains("calc"), "{stdout}");
    assert!(stdout.contains("risky"), "{stdout}");

    // The result shows the tier of the newest version and the hash that
    // names it.
    assert!(stdout.contains("none"), "{stdout}");
    assert!(stdout.contains("L0"), "{stdout}");
    assert!(stdout.contains("L3"), "{stdout}");
    for id in ["guide", "calc", "risky"] {
        let hash = version_hash(&store, id);
        assert!(stdout.contains(&hash), "{stdout}");
    }
}

#[test]
fn the_filter_of_the_tier_keeps_the_versions_that_need_no_more_room() {
    let store = store_fixture("store-search-tier");

    // A tier of L0 keeps the guidance and the skill that only computes,
    // but not the skill that reaches outside of the process.
    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        QUERY,
        "--max-tier",
        "L0",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("guide"), "{stdout}");
    assert!(stdout.contains("calc"), "{stdout}");
    assert!(!stdout.contains("risky"), "{stdout}");

    // A tier of L3 keeps every skill of the store.
    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        QUERY,
        "--max-tier",
        "L3",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("guide"), "{stdout}");
    assert!(stdout.contains("calc"), "{stdout}");
    assert!(stdout.contains("risky"), "{stdout}");
}

#[test]
fn the_safe_filter_keeps_the_versions_that_are_safe_to_run() {
    let store = store_fixture("store-search-safe");

    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        QUERY,
        "--safe",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("guide"), "{stdout}");
    assert!(stdout.contains("calc"), "{stdout}");

    // The skill with a high finding needs a microVM, so it is never safe.
    assert!(!stdout.contains("risky"), "{stdout}");
}

#[test]
fn the_safe_filter_keeps_only_the_versions_that_need_no_container() {
    let store = container_store_fixture("store-search-safe-container");

    // The skill of the tier L2 holds no high finding at all, so it is a
    // result of a search without a filter.
    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        QUERY,
        "--json",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("the json of the search");
    let hits = value.as_array().expect("the hits of the search");
    let container = hits
        .iter()
        .find(|hit| hit["id"] == "container")
        .expect("the hit of the container");
    assert_eq!(container["tier"].as_str(), Some("L2"), "{stdout}");
    assert_eq!(container["high"].as_u64(), Some(0), "{stdout}");

    // The tier L2 needs a container, so a safe search never shows the
    // skill, not even when it holds no high finding at all.
    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        QUERY,
        "--safe",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("guide"), "{stdout}");
    assert!(stdout.contains("calc"), "{stdout}");
    assert!(!stdout.contains("container"), "{stdout}");
}

#[test]
fn the_store_of_a_search_is_a_directory_that_exists() {
    let store = store_fixture("store-search-store-option");
    let root = store
        .parent()
        .expect("the home of the fixture")
        .to_path_buf();

    // A `--store` whose value is another option never names a store.
    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        "--safe",
        QUERY,
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--store"), "{stderr}");

    // A `--store` whose value is a file is not a store either.
    let file = root.join("not-a-store.txt");
    fs::write(&file, "text\n").unwrap();
    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&file),
        QUERY,
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--store"), "{stderr}");

    // A `--store` whose value is not there at all is not a store either.
    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&root.join("no-store-here")),
        QUERY,
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--store"), "{stderr}");
}

#[test]
fn the_json_of_a_search_of_the_store_shows_the_tier_the_hash_and_the_findings() {
    let store = store_fixture("store-search-json");

    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        QUERY,
        "--json",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("the json of the search");
    let hits = value.as_array().expect("the hits of the search");
    assert_eq!(hits.len(), 3, "{stdout}");

    for hit in hits {
        let id = hit["id"].as_str().expect("the id of the hit");
        let tier = hit["tier"].as_str().expect("the tier of the hit");
        let hash = hit["hash"].as_str().expect("the hash of the hit");
        let high = hit["high"].as_u64().expect("the high findings of the hit");
        assert_eq!(hash, version_hash(&store, id), "{stdout}");
        match id {
            "guide" => {
                assert_eq!(tier, "none", "{stdout}");
                assert_eq!(high, 0, "{stdout}");
            }
            "calc" => {
                assert_eq!(tier, "L0", "{stdout}");
                assert_eq!(high, 0, "{stdout}");
            }
            "risky" => {
                assert_eq!(tier, "L3", "{stdout}");
                assert_eq!(high, 1, "{stdout}");
            }
            other => panic!("unexpected skill `{other}` in {stdout}"),
        }
    }
}

#[test]
fn a_search_of_the_store_without_a_match_ends_with_status_one() {
    let store = store_fixture("store-search-empty");

    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        "nothing-matches",
    ]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no skills match"), "{stdout}");
}

#[test]
fn a_search_of_the_store_without_a_store_searches_the_default_store() {
    let store = store_fixture("store-search-default");

    // The store of the environment is the default store of the command.
    let output = run_with_env(
        &["catalog", "search", "--in-store", "guide"],
        &[("SKRUN_STORE", &path(&store))],
    );

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("guide"), "{stdout}");

    // The store below the home directory is the default store as well.
    let home = store
        .parent()
        .expect("the home of the fixture")
        .join("home");
    fs::create_dir_all(&home).unwrap();
    let default = home.join(".skrun").join("store");
    copy_store(&store, &default);

    let output = run_with_env(
        &["catalog", "search", "--in-store", "guide"],
        &[("HOME", &path(&home)), ("SKRUN_STORE", "")],
    );

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("guide"), "{stdout}");
}

#[test]
fn the_store_is_only_searched_when_the_in_store_option_is_given() {
    let store = store_fixture("store-search-mode");

    // The store is only searched with `--in-store`, so a `--store` of a
    // search of a directory is a usage error and never a search of the
    // store.
    let output = run(&["catalog", "search", "--store", &path(&store), QUERY]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--store needs --in-store"), "{stderr}");

    // A search of a directory with a `--store` is a usage error as well.
    let output = run(&[
        "catalog",
        "search",
        "--store",
        &path(&store),
        "skills",
        QUERY,
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--store needs --in-store"), "{stderr}");
}

#[test]
fn a_big_skill_is_still_searched() {
    let store = big_store_fixture("store-search-big");

    // The term is in the body of the skill and the tag is in its
    // frontmatter. The file of the skill is bigger than the limit of
    // `catalog cat`, so a search that used that limit would lose the body
    // and the tags of the skill and would find nothing.
    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        "sprocket",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("big"), "{stdout}");

    // The tag of the frontmatter is a field of the search as well, so a
    // frontmatter of a big file is read as well.
    let output = run(&[
        "catalog",
        "search",
        "--in-store",
        "--store",
        &path(&store),
        "gadget",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("big"), "{stdout}");
}

/// Run the built binary with the given arguments.
fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .output()
        .expect("run the skrun binary")
}

/// Run the built binary with the given environment.
fn run_with_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(BINARY);
    command.args(args);
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("run the skrun binary")
}

/// A store with three skills of three different tiers.
///
/// The guide is guidance and needs no room at all, so its tier is `none`.
/// The calculator only computes, so its tier is `L0`. The runner holds a
/// high finding, so its tier is `L3` and it is never safe to run.
fn store_fixture(name: &str) -> PathBuf {
    let root = temp_dir(name);
    let store = root.join("store");

    let guide = root.join("guide");
    write(
        &guide.join("SKILL.md"),
        concat!(
            "---\n",
            "name: Widget Guide\n",
            "description: A guide that builds a widget.\n",
            "tags:\n",
            "  - widget\n",
            "---\n",
            "\n",
            "# Widget Guide\n",
        ),
    );
    add(&guide, &store);

    let calculator = root.join("calc");
    write(
        &calculator.join("SKILL.md"),
        concat!(
            "---\n",
            "name: Widget Calculator\n",
            "description: Computes the size of a widget.\n",
            "---\n",
            "\n",
            "# Widget Calculator\n",
        ),
    );
    write(&calculator.join("run.py"), "print(sum(range(10)))\n");
    add(&calculator, &store);

    let runner = root.join("risky");
    write(
        &runner.join("SKILL.md"),
        concat!(
            "---\n",
            "name: Widget Runner\n",
            "description: Runs a widget of the system.\n",
            "---\n",
            "\n",
            "# Widget Runner\n",
        ),
    );
    write(
        &runner.join("run.py"),
        "import base64\n\nexec(base64.b64decode(payload))\n",
    );
    add(&runner, &store);

    store
}

/// A store with a skill that needs a container.
///
/// The guide is guidance and needs no room at all, so its tier is `none`.
/// The calculator only computes, so its tier is `L0`. The container uses a
/// module that is not in the standard library, so its tier is `L2` and it
/// holds no high finding at all.
fn container_store_fixture(name: &str) -> PathBuf {
    let root = temp_dir(name);
    let store = root.join("store");

    let guide = root.join("guide");
    write(
        &guide.join("SKILL.md"),
        concat!(
            "---\n",
            "name: Widget Guide\n",
            "description: A guide that builds a widget.\n",
            "---\n",
            "\n",
            "# Widget Guide\n",
        ),
    );
    add(&guide, &store);

    let calculator = root.join("calc");
    write(
        &calculator.join("SKILL.md"),
        concat!(
            "---\n",
            "name: Widget Calculator\n",
            "description: Computes the size of a widget.\n",
            "---\n",
            "\n",
            "# Widget Calculator\n",
        ),
    );
    write(&calculator.join("run.py"), "print(sum(range(10)))\n");
    add(&calculator, &store);

    let container = root.join("container");
    write(
        &container.join("SKILL.md"),
        concat!(
            "---\n",
            "name: Widget Container\n",
            "description: Uses a module of the outside to build a widget.\n",
            "---\n",
            "\n",
            "# Widget Container\n",
        ),
    );
    write(
        &container.join("run.py"),
        "import requests\n\nprint(requests)\n",
    );
    add(&container, &store);

    store
}

/// A store with one skill whose `SKILL.md` is bigger than the limit of
/// `catalog cat`.
///
/// The body of the skill holds the term at its end, so the term is only
/// found when the whole file is read.
fn big_store_fixture(name: &str) -> PathBuf {
    let root = temp_dir(name);
    let store = root.join("store");

    let skill = root.join("big");
    let mut text = String::new();
    text.push_str("---\n");
    text.push_str("name: Big Guide\n");
    text.push_str("description: A guide that is very long.\n");
    text.push_str("tags: [gadget]\n");
    text.push_str("---\n\n");
    // The filler is bigger than the limit of `catalog cat`, so the term
    // below is only read when the limit of a search is used.
    let filler = "The quick brown fox jumps over the lazy dog.\n";
    while text.len() < 3 * 1000 * 1000 {
        text.push_str(filler);
    }
    text.push_str("\n# Sprocket\n");
    write(&skill.join("SKILL.md"), &text);
    add(&skill, &store);

    store
}

/// Add a skill to the store and fail when the add fails.
fn add(source: &Path, store: &Path) {
    let output = run(&["catalog", "add", &path(source), "--store", &path(store)]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

/// The hash that names the newest version of a stored skill.
fn version_hash(store: &Path, id: &str) -> String {
    let record = fs::read_to_string(store.join("records").join(format!("{id}.json")))
        .expect("the record of the stored skill");
    let value: serde_json::Value = serde_json::from_str(&record).expect("the record of the skill");
    value["versions"]
        .as_array()
        .expect("the versions of the skill")
        .last()
        .expect("the newest version of the skill")["hash"]
        .as_str()
        .expect("the hash of the newest version")
        .to_string()
}

/// Copy a store into another folder.
fn copy_store(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_store(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// The path of a folder as a string.
fn path(folder: &Path) -> String {
    folder.to_string_lossy().into_owned()
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
        "skrun-catalog-store-search-{name}-{}-{now}-{count}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
