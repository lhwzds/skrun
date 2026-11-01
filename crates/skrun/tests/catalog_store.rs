//! The catalog store commands never run a skill and never print a secret.
//!
//! The tests run the built `skrun` binary as a subprocess, so they cover the
//! real command output: stdout and stderr together, exactly as a user sees
//! them. Every fixture is only ever copied, hashed and scanned, never run.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// A key shaped value that must never reach the command output.
const SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn a_directory_skill_is_stored_and_the_secret_stays_out_of_the_output() {
    let source = skill_fixture("store-directory");
    let store = temp_dir("store-directory-store");

    let output = run(&[
        "catalog",
        "add",
        &path(&source),
        "--store",
        &path(&store),
        "--from",
        "https://example.com/repo",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("stored demo "), "{stdout}");
    assert!(stdout.contains(" (2 files, 1 findings)"), "{stdout}");
    assert_clean(&output);

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(record.contains("\"rule\": \"secret\""), "{record}");
    assert!(record.contains("\"line\": 3"), "{record}");
    assert!(record.contains("\"path\": \"run.py\""), "{record}");
    assert!(record.contains("\"language\": \"python\""), "{record}");
    assert!(!record.contains(SECRET), "{record}");

    let hash = stdout
        .split_whitespace()
        .nth(2)
        .expect("the hash of the stored skill")
        .to_string();
    let stored = store.join("skills/demo").join(&hash);
    assert!(stored.join("SKILL.md").is_file());
    assert!(stored.join("run.py").is_file());
}

#[test]
fn the_stored_copy_is_read_only() {
    let source = skill_fixture("store-read-only");
    let store = temp_dir("store-read-only-store");

    let output = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hash = stdout.split_whitespace().nth(2).unwrap().to_string();
    let stored = store.join("skills/demo").join(&hash);

    assert_eq!(mode_of(&stored.join("run.py")), 0o400);
    assert_eq!(mode_of(&stored.join("SKILL.md")), 0o400);
    assert_eq!(mode_of(&stored), 0o500);
}

#[test]
fn the_store_keeps_its_state_to_itself() {
    let source = skill_fixture("store-private-store");
    let store = temp_dir("store-private-store-store");

    let output = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(mode_of(&store), 0o700);
    assert_eq!(mode_of(&store.join("skills")), 0o700);
    assert_eq!(mode_of(&store.join("records")), 0o700);
    assert_eq!(mode_of(&store.join("locks")), 0o700);
    assert_eq!(mode_of(&store.join("records/demo.json")), 0o600);
    assert_clean(&output);
}

#[test]
fn the_same_skill_is_already_stored() {
    let source = skill_fixture("store-again");
    let store = temp_dir("store-again-store");

    let first = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(first.status.code(), Some(0));

    let second = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(second.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.starts_with("already stored demo "), "{stdout}");
    assert!(stdout.contains(" (2 files, 1 findings)"), "{stdout}");
    assert_clean(&second);

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert_eq!(record.matches("\"full_hash\"").count(), 1, "{record}");
}

#[test]
fn an_add_of_the_same_content_refreshes_the_record() {
    let source = skill_fixture("store-refresh");
    let store = temp_dir("store-refresh-store");

    let first = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(first.status.code(), Some(0));
    let record_path = store.join("records/demo.json");
    let before: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    let before_version = before["versions"][0].clone();

    // The record of an older rule holds the wrong findings and the wrong tier.
    let mut stale = before.clone();
    stale["versions"][0]["findings"] = serde_json::json!([]);
    stale["versions"][0]["tier"] = serde_json::json!("L3");
    stale["versions"][0]["tier_reasons"] = serde_json::json!(["an old rule"]);
    write(&record_path, &serde_json::to_string_pretty(&stale).unwrap());

    let second = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);

    assert_eq!(second.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("already stored (record refreshed)"),
        "{stdout}"
    );
    assert_clean(&second);

    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(after["versions"].as_array().unwrap().len(), 1);
    let after_version = &after["versions"][0];
    assert_eq!(after_version["hash"], before_version["hash"]);
    assert_eq!(after_version["from"], before_version["from"]);
    assert_eq!(after_version["added_at"], before_version["added_at"]);
    assert_eq!(after_version["findings"], before_version["findings"]);
    assert_eq!(after_version["tier"], before_version["tier"]);
    assert_eq!(
        after_version["tier_reasons"],
        before_version["tier_reasons"]
    );
}

#[test]
fn a_changed_file_adds_a_second_version() {
    let source = skill_fixture("store-version");
    let store = temp_dir("store-version-store");

    let first = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(first.status.code(), Some(0));
    let first_hash = String::from_utf8_lossy(&first.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    write(&source.join("run.py"), "import os\n\nprint('hello')\n");
    let second = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(second.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&second.stdout);
    let second_hash = stdout.split_whitespace().nth(2).unwrap().to_string();
    assert_ne!(first_hash, second_hash);
    assert!(stdout.starts_with("stored demo "), "{stdout}");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert_eq!(record.matches("\"full_hash\"").count(), 2, "{record}");
}

#[test]
fn a_zip_with_a_parent_path_is_refused() {
    let store = temp_dir("store-zip-parent-store");
    let archive = temp_dir("store-zip-parent").join("evil.zip");
    write_zip(&archive, &[("../evil", "boom")]);

    let output = run(&["catalog", "add", &path(&archive), "--store", &path(&store)]);

    assert_ne!(output.status.code(), Some(0));
    assert!(!store.join("skills").exists());
    assert!(!store.join("records").exists());
    assert_clean(&output);
}

#[test]
fn a_zip_with_a_symlink_is_refused() {
    let store = temp_dir("store-zip-symlink-store");
    let archive = temp_dir("store-zip-symlink").join("link.zip");
    {
        let file = fs::File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .add_symlink(
                "link",
                "/etc/passwd",
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.finish().unwrap();
    }

    let output = run(&["catalog", "add", &path(&archive), "--store", &path(&store)]);

    assert_ne!(output.status.code(), Some(0));
    assert!(!store.join("skills").exists());
    assert!(!store.join("records").exists());
    assert_clean(&output);
}

#[test]
fn a_zip_with_too_many_entries_is_refused() {
    let store = temp_dir("store-zip-many-store");
    let archive = temp_dir("store-zip-many").join("many.zip");
    {
        let file = fs::File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for index in 0..=5_000 {
            writer
                .start_file(format!("file-{index}.txt"), options)
                .unwrap();
            writer.write_all(b"x").unwrap();
        }
        writer.finish().unwrap();
    }

    let output = run(&["catalog", "add", &path(&archive), "--store", &path(&store)]);

    assert_ne!(output.status.code(), Some(0));
    assert!(!store.join("skills").exists());
    assert!(!store.join("records").exists());
    assert_clean(&output);
}

#[test]
fn a_zip_with_one_top_folder_is_rooted_at_that_folder() {
    let store = temp_dir("store-zip-folder-store");
    let archive = temp_dir("store-zip-folder").join("zipped.zip");
    write_zip(
        &archive,
        &[
            ("my-skill/SKILL.md", "---\nname: zipped\n---\n\n# Zipped\n"),
            ("my-skill/run.py", "import sys\n"),
        ],
    );

    let output = run(&["catalog", "add", &path(&archive), "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("stored zipped "), "{stdout}");
    assert!(stdout.contains(" (2 files, 0 findings)"), "{stdout}");

    let hash = stdout.split_whitespace().nth(2).unwrap().to_string();
    let stored = store.join("skills/zipped").join(&hash);
    assert!(stored.join("SKILL.md").is_file());
    assert!(stored.join("run.py").is_file());
}

#[test]
fn two_skills_with_the_same_frontmatter_name_are_two_records() {
    let root = temp_dir("store-two-skills");
    let first = root.join("a-skill");
    let second = root.join("b-skill");
    for source in [&first, &second] {
        write(
            &source.join("SKILL.md"),
            "---\nname: same\ndescription: A shared name.\n---\n\n# Same\n",
        );
    }
    let store = temp_dir("store-two-skills-store");

    let one = run(&["catalog", "add", &path(&first), "--store", &path(&store)]);
    let two = run(&["catalog", "add", &path(&second), "--store", &path(&store)]);

    assert_eq!(one.status.code(), Some(0));
    assert_eq!(two.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&one.stdout).starts_with("stored a-skill "),
        "{}",
        String::from_utf8_lossy(&one.stdout)
    );
    assert!(
        String::from_utf8_lossy(&two.stdout).starts_with("stored b-skill "),
        "{}",
        String::from_utf8_lossy(&two.stdout)
    );
    assert!(store.join("records/a-skill.json").is_file());
    assert!(store.join("records/b-skill.json").is_file());
    assert_clean(&one);
    assert_clean(&two);
}

#[test]
fn a_directory_with_a_valid_name_stores_under_its_name() {
    let source = temp_dir("store-good-id-source").join("good-id");
    write(
        &source.join("SKILL.md"),
        "---\nname: \u{547d}\u{7406}\u{5927}\u{5e08}\ndescription: A master.\n---\n\n# Master\n",
    );
    let store = temp_dir("store-good-id-store");

    let output = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("stored good-id "), "{stdout}");
    assert!(store.join("records/good-id.json").is_file());
    assert_clean(&output);
}

#[test]
fn the_current_folder_is_stored_under_the_name_of_the_folder() {
    let source = skill_fixture("store-current-folder");
    let store = temp_dir("store-current-folder-store");

    // The source is the folder the command runs in, so the skill is named
    // after the folder and not after the `.` the user typed.
    let output = Command::new(BINARY)
        .args(["catalog", "add", ".", "--store", &path(&store)])
        .current_dir(&source)
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("stored demo "), "{stdout}");
    assert!(store.join("records/demo.json").is_file());
    assert_clean(&output);
}

#[test]
fn the_parent_folder_is_stored_under_the_name_of_the_folder() {
    let source = skill_fixture("store-parent-folder");
    let store = temp_dir("store-parent-folder-store");
    let parent = source.parent().unwrap().to_path_buf();
    let name = parent.file_name().unwrap().to_string_lossy().into_owned();

    // The source is the parent of the folder the command runs in.
    let output = Command::new(BINARY)
        .args(["catalog", "add", "..", "--store", &path(&store)])
        .current_dir(&source)
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with(&format!("stored {name} ")), "{stdout}");
    assert!(store.join("records").join(format!("{name}.json")).is_file());
    assert_clean(&output);
}

#[test]
fn a_folder_that_is_written_with_a_path_is_stored_under_its_name() {
    let root = temp_dir("store-relative-folder");
    let source = root.join("my-skill");
    write(
        &source.join("SKILL.md"),
        "---\nname: my-skill\ndescription: A skill.\n---\n\n# My Skill\n",
    );
    let store = temp_dir("store-relative-folder-store");

    // The source is written as a path that ends in a slash.
    let output = Command::new(BINARY)
        .args(["catalog", "add", "./my-skill/", "--store", &path(&store)])
        .current_dir(&root)
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("stored my-skill "), "{stdout}");
    assert!(store.join("records/my-skill.json").is_file());
    assert_clean(&output);
}

#[test]
fn a_zip_is_stored_under_the_stem_of_its_name() {
    let store = temp_dir("store-zip-stem-store");
    let archive = temp_dir("store-zip-stem").join("x.zip");
    write_zip(
        &archive,
        &[("SKILL.md", "---\nname: other\n---\n\n# Other\n")],
    );

    let output = run(&["catalog", "add", &path(&archive), "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("stored x "), "{stdout}");
    assert!(store.join("records/x.json").is_file());
    assert_clean(&output);
}

#[test]
fn a_source_that_is_no_id_asks_for_an_id() {
    let source = temp_dir("store-bad-id-source").join("\u{547d}\u{7406}\u{5927}\u{5e08}");
    write(
        &source.join("SKILL.md"),
        "---\nname: \u{547d}\u{7406}\u{5927}\u{5e08}\ndescription: A master.\n---\n\n# Master\n",
    );
    let store = temp_dir("store-bad-id-store");

    let output = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);

    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--id"), "{stderr}");
    assert!(!store.join("records").exists());
    assert_clean(&output);
}

#[test]
fn the_named_id_wins_over_the_source() {
    let source = temp_dir("store-named-id-source").join("a-skill");
    write(
        &source.join("SKILL.md"),
        "---\nname: same\ndescription: A shared name.\n---\n\n# Same\n",
    );
    let store = temp_dir("store-named-id-store");

    let output = run(&[
        "catalog",
        "add",
        &path(&source),
        "--store",
        &path(&store),
        "--id",
        "named-id",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("stored named-id "), "{stdout}");
    assert!(store.join("records/named-id.json").is_file());
    assert_clean(&output);
}

#[test]
fn a_file_with_elf_magic_is_a_binary_finding() {
    let source = skill_fixture("store-binary");
    let store = temp_dir("store-binary-store");
    fs::write(source.join("tool.bin"), b"\x7fELF\x02\x01\x01\x00rest").unwrap();

    let output = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(" (3 files, 2 findings)"), "{stdout}");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(record.contains("\"rule\": \"binary\""), "{record}");
    assert!(record.contains("\"severity\": \"high\""), "{record}");
    assert!(record.contains("\"path\": \"tool.bin\""), "{record}");
    assert_clean(&output);
}

#[test]
fn the_inspect_command_shows_the_record_and_the_list() {
    let source = skill_fixture("store-inspect");
    let store = temp_dir("store-inspect-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let list = run(&["catalog", "inspect", "--list", "--store", &path(&store)]);
    assert_eq!(list.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("demo"), "{stdout}");
    assert!(stdout.contains("tier L1"), "{stdout}");
    assert!(stdout.contains("high 0"), "{stdout}");
    assert!(stdout.contains("medium 1"), "{stdout}");
    assert!(stdout.contains("low 0"), "{stdout}");
    assert_clean(&list);

    let record = run(&["catalog", "inspect", "demo", "--store", &path(&store)]);
    assert_eq!(record.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&record.stdout);
    assert!(stdout.contains("version "), "{stdout}");
    assert!(stdout.contains("run.py"), "{stdout}");
    assert!(stdout.contains("python"), "{stdout}");
    assert!(stdout.contains("medium secret run.py:3"), "{stdout}");
    assert_clean(&record);

    let json = run(&[
        "catalog",
        "inspect",
        "demo",
        "--store",
        &path(&store),
        "--json",
    ]);
    assert_eq!(json.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&json.stdout);
    assert!(stdout.contains("\"schema_version\": 1"), "{stdout}");
    assert!(stdout.contains("\"full_hash\""), "{stdout}");
    assert!(stdout.contains("\"rule\": \"secret\""), "{stdout}");
    assert_clean(&json);
}

#[test]
fn an_unknown_id_fails_with_status_one() {
    let store = temp_dir("store-unknown-store");

    let output = run(&["catalog", "inspect", "nothing", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(1));
    assert_clean(&output);
}

#[test]
fn the_store_root_follows_the_environment_variable() {
    let source = skill_fixture("store-environment");
    let store = temp_dir("store-environment-store");

    let output = Command::new(BINARY)
        .args(["catalog", "add", &path(&source)])
        .env("SKRUN_STORE", &store)
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    assert!(store.join("records/demo.json").is_file());
    assert_clean(&output);
}

#[test]
fn a_source_that_is_a_symlink_is_refused() {
    let source = skill_fixture("store-symlink-source");
    let store = temp_dir("store-symlink-source-store");
    let link = temp_dir("store-symlink-source-link").join("link");
    std::os::unix::fs::symlink(&source, &link).unwrap();

    let output = run(&["catalog", "add", &path(&link), "--store", &path(&store)]);

    assert_ne!(output.status.code(), Some(0));
    assert!(!store.join("skills").exists());
    assert!(!store.join("records").exists());
    assert_clean(&output);
}

#[test]
fn a_source_with_more_than_one_top_level_entry_is_refused() {
    let source = temp_dir("store-two-entries");
    write(
        &source.join("my-skill/SKILL.md"),
        "---\nname: nested\n---\n\n# Nested\n",
    );
    write(&source.join("README.md"), "# Readme\n");
    let store = temp_dir("store-two-entries-store");

    let output = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);

    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("SKILL.md must be at the top level or inside a single top-level folder"),
        "{stderr}"
    );
    assert!(!store.join("skills").exists());
    assert!(!store.join("records").exists());
    assert_clean(&output);
}

#[test]
fn a_secret_in_a_file_that_is_not_text_is_found() {
    let source = skill_fixture("store-binary-secret");
    let store = temp_dir("store-binary-secret-store");
    let mut content = b"\x00\x01\x02payload\n".to_vec();
    content.extend_from_slice(format!("KEY = \"{SECRET}\"\n").as_bytes());
    fs::write(source.join("payload.dat"), &content).unwrap();

    let output = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(record.contains("\"rule\": \"secret\""), "{record}");
    assert!(record.contains("\"path\": \"payload.dat\""), "{record}");
    assert!(record.contains("\"line\": 2"), "{record}");
    assert_clean(&output);
}

#[test]
fn a_stored_copy_without_a_record_repairs_the_record() {
    let source = skill_fixture("store-repair");
    let store = temp_dir("store-repair-store");

    let first = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(first.status.code(), Some(0));
    fs::remove_file(store.join("records/demo.json")).unwrap();

    let second = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);

    assert_eq!(second.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.starts_with("stored demo "), "{stdout}");
    assert_clean(&second);

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert_eq!(record.matches("\"full_hash\"").count(), 1, "{record}");
}

#[test]
fn two_adds_at_the_same_time_store_one_version() {
    let source = skill_fixture("store-concurrent");
    let store = temp_dir("store-concurrent-store");

    let first = Command::new(BINARY)
        .args(["catalog", "add", &path(&source), "--store", &path(&store)])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the first add");
    let second = Command::new(BINARY)
        .args(["catalog", "add", &path(&source), "--store", &path(&store)])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the second add");
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();

    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));
    assert_clean(&first);
    assert_clean(&second);

    let stdout = format!(
        "{}{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&second.stdout)
    );
    assert!(stdout.contains("already stored demo "), "{stdout}");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert_eq!(record.matches("\"full_hash\"").count(), 1, "{record}");
    assert!(!store.join("locks/demo.lock").exists());
}

#[test]
fn a_zip_entry_with_a_name_that_is_not_utf8_is_refused() {
    let store = temp_dir("store-utf8-store");
    let archive = temp_dir("store-utf8").join("bad.zip");
    write_zip_with_raw_name(&archive, b"bad-\xff-name.txt", b"hello\n");

    let output = run(&["catalog", "add", &path(&archive), "--store", &path(&store)]);

    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not valid UTF-8"), "{stderr}");
    assert!(!store.join("skills").exists());
    assert!(!store.join("records").exists());
    assert_clean(&output);
}

#[test]
fn the_inspect_command_redacts_every_value_of_the_record() {
    let store = temp_dir("store-redact-store");
    let record = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"id\": \"demo\",\n",
            "  \"versions\": [\n",
            "    {{\n",
            "      \"hash\": \"0123456789abcdef\",\n",
            "      \"full_hash\": \"0123456789abcdef0123456789abcdef\",\n",
            "      \"from\": \"{secret}\",\n",
            "      \"added_at\": \"2026-01-01T00:00:00Z\",\n",
            "      \"files\": 2,\n",
            "      \"bytes\": 10,\n",
            "      \"name\": \"{secret}\",\n",
            "      \"description\": \"{secret}\",\n",
            "      \"scripts\": [{{ \"path\": \"{secret}\", \"language\": \"python\", ",
            "\"imports\": [\"{secret}\"] }}],\n",
            "      \"findings\": [{{ \"rule\": \"secret\", \"severity\": \"high\", ",
            "\"path\": \"{secret}\", \"line\": 3 }}]\n",
            "    }}\n",
            "  ]\n",
            "}}\n"
        ),
        secret = SECRET
    );
    write(&store.join("records/demo.json"), &record);

    let json = run(&[
        "catalog",
        "inspect",
        "demo",
        "--store",
        &path(&store),
        "--json",
    ]);
    assert_eq!(json.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&json.stdout);
    assert!(!stdout.contains(SECRET), "{stdout}");
    assert!(stdout.contains("<redacted>"), "{stdout}");

    let text = run(&["catalog", "inspect", "demo", "--store", &path(&store)]);
    assert_eq!(text.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(!stdout.contains(SECRET), "{stdout}");
    assert!(stdout.contains("<redacted>"), "{stdout}");
    assert_clean(&json);
    assert_clean(&text);
}

/// Run the `skrun` binary with the given arguments.
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

/// A skill directory with a python script and a planted key.
///
/// The skill lives in a folder named `demo`, because the id of a skill comes
/// from the name of its source.
fn skill_fixture(name: &str) -> PathBuf {
    let dir = temp_dir(name).join("demo");
    write(
        &dir.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    write(
        &dir.join("run.py"),
        "import os\n\nKEY = \"AKIAIOSFODNN7EXAMPLE\"\nprint(KEY)\n",
    );
    dir
}

/// A zip archive with the given entries.
fn write_zip(path: &Path, entries: &[(&str, &str)]) {
    let file = fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, content) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
}

/// A zip archive with one entry whose name holds bytes that are not UTF-8.
///
/// The archive is written by hand, because a zip writer only holds names
/// that are text.
fn write_zip_with_raw_name(path: &Path, name: &[u8], content: &[u8]) {
    let crc = crc32(content);
    let size = content.len() as u32;
    let length = name.len() as u16;

    let mut local = Vec::new();
    local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    local.extend_from_slice(&20u16.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&crc.to_le_bytes());
    local.extend_from_slice(&size.to_le_bytes());
    local.extend_from_slice(&size.to_le_bytes());
    local.extend_from_slice(&length.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(name);
    local.extend_from_slice(content);

    let mut central = Vec::new();
    central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    central.extend_from_slice(&20u16.to_le_bytes());
    central.extend_from_slice(&20u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&crc.to_le_bytes());
    central.extend_from_slice(&size.to_le_bytes());
    central.extend_from_slice(&size.to_le_bytes());
    central.extend_from_slice(&length.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u32.to_le_bytes());
    central.extend_from_slice(&0u32.to_le_bytes());
    central.extend_from_slice(name);

    let mut archive = local;
    let offset = archive.len() as u32;
    archive.extend_from_slice(&central);
    let size = archive.len() as u32 - offset;
    archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&1u16.to_le_bytes());
    archive.extend_from_slice(&1u16.to_le_bytes());
    archive.extend_from_slice(&size.to_le_bytes());
    archive.extend_from_slice(&offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    fs::write(path, archive).unwrap();
}

/// The CRC32 of some bytes, as the zip format wants it.
fn crc32(content: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in content {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = if crc & 1 == 1 { 0xedb8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    !crc
}

/// The mode of a file or of a directory.
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The text of a path.
fn path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
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
        "skrun-store-test-{name}-{}-{now}-{count}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
