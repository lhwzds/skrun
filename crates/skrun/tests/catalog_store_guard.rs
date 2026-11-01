//! The store never trusts what it cannot read and never trusts what it
//! cannot tell apart.
//!
//! Every test writes a fixture and adds the fixture to a store. A fixture
//! is only ever copied, hashed and read: nothing of a fixture is ever run.

use skrun::store::{self, AddOptions, Severity, StoredVersion};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

/// The number of bytes of a file that the risk rules do not read.
const SCAN_LIMIT: usize = 16 * 1024 * 1024;

#[test]
fn two_skills_with_the_same_old_hash_are_two_versions() {
    // The old hash joined the path and the content of every file with a
    // null byte, so the file `a` with the content `\0b\0c` and the files
    // `a` and `b` held the same bytes and the same hash. The hash of today
    // writes the length of every path and of every content in front of it,
    // so the two skills are two versions.
    let root = temp_dir("guard-hash");
    let first = root.join("first/demo");
    let second = root.join("second/demo");
    write(
        &first.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    write(&first.join("a"), "\u{0}b\u{0}c");
    write(
        &second.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    write(&second.join("a"), "");
    write(&second.join("b"), "c");
    let store = temp_dir("guard-hash-store");

    let one = add(&first, &store);
    let two = add(&second, &store);

    assert_ne!(one.hash, two.hash, "the two skills hold different hashes");
    assert!(store.join("skills/demo").join(&one.hash).is_dir());
    assert!(store.join("skills/demo").join(&two.hash).is_dir());

    let record = store::read_record(&store, "demo").unwrap();
    assert_eq!(record.versions.len(), 2, "{record:?}");
    assert_ne!(record.versions[0].full_hash, record.versions[1].full_hash);
}

#[test]
fn a_version_that_shares_the_short_hash_is_stored_under_a_longer_name() {
    let source = skill_fixture("guard-short-hash", &[("run.py", "import os\n")]);
    let store = temp_dir("guard-short-hash-store");

    let version = add(&source, &store);
    let folder = store.join("skills/demo").join(&version.hash);

    // The folder of the short hash holds another version, so the version of
    // today is stored under a longer name and the two versions never share
    // a folder.
    fs::set_permissions(
        &folder,
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
    )
    .unwrap();
    fs::write(folder.join("other.txt"), "other\n").unwrap();
    let changed = add(&source, &store);

    assert_ne!(changed.hash, version.hash);
    assert!(changed.hash.len() > version.hash.len());
    assert!(store.join("skills/demo").join(&changed.hash).is_dir());
}

#[test]
fn a_script_that_is_too_big_to_be_read_is_the_third_tier() {
    let mut padding = String::with_capacity(SCAN_LIMIT + 1024);
    padding.push('#');
    padding.push_str(&"a".repeat(SCAN_LIMIT + 1024));
    padding.push('\n');
    let script = format!("import base64\n\nexec(base64.b64decode(payload))\n{padding}");
    let source = skill_fixture("guard-big", &[("run.py", script.as_str())]);
    let store = temp_dir("guard-big-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "unscanned")
        .expect("an unscanned finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.py");
    assert_eq!(version.tier, "L3");
    assert!(
        version
            .tier_reasons
            .iter()
            .any(|reason| reason.contains("unscanned script")),
        "{:?}",
        version.tier_reasons
    );
    // The file was not read, so the rule of the decoder never ran.
    assert!(
        !version
            .findings
            .iter()
            .any(|finding| finding.rule == "decode_exec"),
        "{:?}",
        version.findings
    );
}

#[test]
fn a_big_file_that_is_no_script_still_needs_a_container() {
    let padding = "a".repeat(SCAN_LIMIT + 1024);
    let source = skill_fixture("guard-big-notes", &[("notes.txt", padding.as_str())]);
    let store = temp_dir("guard-big-notes-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "unscanned")
        .expect("an unscanned finding");
    assert_eq!(finding.path, "notes.txt");
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_big_script_without_an_extension_is_the_third_tier() {
    let padding = "a".repeat(SCAN_LIMIT + 1024);
    let script = format!("#!/bin/sh\n\necho hello\n{padding}");
    let source = skill_fixture("guard-big-shebang", &[("run", script.as_str())]);
    let store = temp_dir("guard-big-shebang-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "unscanned")
        .expect("an unscanned finding");
    assert_eq!(finding.path, "run");
    assert_eq!(version.tier, "L3");
}

#[test]
fn an_import_behind_a_colon_is_an_import() {
    let source = skill_fixture(
        "guard-import-colon",
        &[(
            "run.py",
            "if True: import subprocess\n\nprint(subprocess)\n",
        )],
    );
    let store = temp_dir("guard-import-colon-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run.py")
        .expect("a script");
    assert!(
        script.imports.contains(&"subprocess".to_string()),
        "{script:?}"
    );
    assert_eq!(version.tier, "L2");
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
fn an_import_of_a_try_is_an_import() {
    let source = skill_fixture(
        "guard-import-try",
        &[(
            "run.py",
            "try: import socket\n\nexcept OSError:\n    pass\n",
        )],
    );
    let store = temp_dir("guard-import-try-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run.py")
        .expect("a script");
    assert!(script.imports.contains(&"socket".to_string()), "{script:?}");
    assert_eq!(version.tier, "L2");
}

#[test]
fn an_import_of_a_call_is_an_import() {
    let source = skill_fixture(
        "guard-import-call",
        &[(
            "run.py",
            "module = __import__(\"socket\")\n\nprint(module)\n",
        )],
    );
    let store = temp_dir("guard-import-call-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run.py")
        .expect("a script");
    assert!(script.imports.contains(&"socket".to_string()), "{script:?}");
    assert_eq!(version.tier, "L2");
}

#[test]
fn an_import_of_the_module_loader_is_an_import() {
    let source = skill_fixture(
        "guard-import-loader",
        &[(
            "run.py",
            "import importlib\n\nmodule = importlib.import_module('subprocess')\n",
        )],
    );
    let store = temp_dir("guard-import-loader-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run.py")
        .expect("a script");
    assert!(
        script.imports.contains(&"subprocess".to_string()),
        "{script:?}"
    );
    assert_eq!(version.tier, "L2");
}

#[test]
fn an_import_in_an_f_string_is_an_import() {
    // The expression of an f-string is code, so the call of the loader runs
    // when the string is built.
    let source = skill_fixture(
        "guard-fstring",
        &[(
            "run.py",
            "value = f\"{__import__('socket')}\"\n\nprint(value)\n",
        )],
    );
    let store = temp_dir("guard-fstring-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run.py")
        .expect("a script");
    assert!(script.imports.contains(&"socket".to_string()), "{script:?}");
    assert_eq!(version.tier, "L2");
}

#[test]
fn an_import_after_a_backslash_in_an_f_string_is_an_import() {
    // The backslash is text of the string, so the brace still opens the
    // expression and the call of the loader runs.
    let source = skill_fixture(
        "guard-fstring-backslash",
        &[(
            "run.py",
            "value = rf\"\\{__import__('socket')}\"\n\nprint(value)\n",
        )],
    );
    let store = temp_dir("guard-fstring-backslash-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run.py")
        .expect("a script");
    assert!(script.imports.contains(&"socket".to_string()), "{script:?}");
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_string_that_runs_over_a_line_needs_a_container() {
    // The string of the file is never closed, so the import that follows
    // the newline is text of the string and the file cannot be tokenised.
    let source = skill_fixture(
        "guard-string-newline",
        &[("run.py", "value = \"abc\nimport subprocess\ndef\"\n")],
    );
    let store = temp_dir("guard-string-newline-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "unparsed")
        .expect("an unparsed finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.py");
    assert_eq!(version.tier, "L2");
}

#[test]
fn an_f_string_with_a_single_brace_needs_a_container() {
    // The interpreter refuses an f-string that holds a single brace.
    let source = skill_fixture("guard-fstring-brace", &[("run.py", "value = f\"}\"\n")]);
    let store = temp_dir("guard-fstring-brace-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "unparsed")
        .expect("an unparsed finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.py");
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_file_that_the_interpreter_refuses_needs_a_container() {
    // The file ends with a line that goes on with a line that never comes,
    // so the tokenizer of the interpreter refuses the file.
    let source = skill_fixture("guard-tokenizer", &[("run.py", "x = 1 \\\n")]);
    let store = temp_dir("guard-tokenizer-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "unparsed")
        .expect("an unparsed finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.py");
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_python_file_that_cannot_be_parsed_needs_a_container() {
    let source = skill_fixture(
        "guard-unparsed",
        &[("run.py", "value = 'unterminated\nprint(value)\n")],
    );
    let store = temp_dir("guard-unparsed-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "unparsed")
        .expect("an unparsed finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.py");
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_file_that_hides_a_module_of_the_standard_library_needs_a_container() {
    let source = skill_fixture(
        "guard-shadow",
        &[
            ("logging.py", "def help():\n    return 1\n"),
            ("run.py", "import json\n\nprint(json)\n"),
        ],
    );
    let store = temp_dir("guard-shadow-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "shadows_stdlib")
        .expect("a shadowing finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "logging.py");
    // The finding keeps the name of the module that the file hides.
    assert_eq!(finding.detail.as_deref(), Some("logging"));
    assert_eq!(version.tier, "L2");
    assert!(
        version
            .tier_reasons
            .iter()
            .any(|reason| reason.contains("shadows the module logging")),
        "{:?}",
        version.tier_reasons
    );
}

#[test]
fn a_file_that_hides_a_risky_module_needs_a_micro_vm() {
    // The file of the skill hides a module that reaches outside of the
    // process, so the interpreter runs the file of the skill where it would
    // run the module of the interpreter.
    let source = skill_fixture(
        "guard-shadow-risky",
        &[
            ("socket.py", "def help():\n    return 1\n"),
            ("run.py", "import json\n\nprint(json)\n"),
        ],
    );
    let store = temp_dir("guard-shadow-risky-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "shadows_stdlib")
        .expect("a shadowing finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "socket.py");
    assert_eq!(finding.detail.as_deref(), Some("socket"));
    assert_eq!(version.tier, "L3");
    assert!(
        version
            .tier_reasons
            .iter()
            .any(|reason| reason.contains("shadows the module socket")),
        "{:?}",
        version.tier_reasons
    );
}

#[test]
fn a_script_without_an_extension_is_a_script() {
    let source = skill_fixture("guard-shebang", &[("run", "#!/bin/sh\n\necho hello\n")]);
    let store = temp_dir("guard-shebang-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run")
        .expect("a script");
    assert_eq!(script.language, "shell");
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_script_of_an_unknown_interpreter_needs_a_container() {
    let source = skill_fixture(
        "guard-shebang-other",
        &[("run", "#!/usr/bin/perl\n\nprint 1;\n")],
    );
    let store = temp_dir("guard-shebang-other-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run")
        .expect("a script");
    assert_eq!(script.language, "other");
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_script_of_a_python_interpreter_is_a_python_script() {
    let source = skill_fixture(
        "guard-shebang-python",
        &[(
            "run",
            "#!/usr/bin/env python3\n\nimport math\n\nprint(math.pi)\n",
        )],
    );
    let store = temp_dir("guard-shebang-python-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run")
        .expect("a script");
    assert_eq!(script.language, "python");
    assert!(script.imports.contains(&"math".to_string()), "{script:?}");
}

#[test]
fn a_password_of_a_description_is_never_printed() {
    let source = temp_dir("guard-secret").join("demo");
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: \"Demo password: hunter2xx\"\n---\n\n# Demo\n",
    );
    let store = temp_dir("guard-secret-store");

    let added = Command::new(BINARY)
        .args([
            "catalog",
            "add",
            &path(&source),
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(added.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&added.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");
    assert!(stdout.contains("<redacted>"), "stdout: {stdout}");

    let inspected = Command::new(BINARY)
        .args([
            "catalog",
            "inspect",
            "demo",
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(inspected.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&inspected.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");
    assert!(stdout.contains("<redacted>"), "stdout: {stdout}");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(!record.contains("hunter2xx"), "record: {record}");
}

#[test]
fn a_zip_that_lies_about_its_size_is_refused() {
    let store = temp_dir("guard-zip-store");
    let archive = temp_dir("guard-zip").join("liar.zip");
    // The archive holds a megabyte of bytes and says that it holds ten.
    write_zip_with_a_small_size(&archive, "SKILL.md", &vec![b'a'; 1024 * 1024]);
    write_zip_with_a_small_size(&archive, "big.txt", &vec![b'b'; 1024 * 1024]);

    let output = Command::new(BINARY)
        .args(["catalog", "add", &path(&archive), "--store", &path(&store)])
        .output()
        .expect("run the skrun binary");

    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("compression ratio") || stderr.contains("more than"),
        "stderr: {stderr}"
    );
    assert!(!store.join("skills").exists());
    assert!(!store.join("records").exists());
}

#[test]
fn a_refused_zip_leaves_no_staging_behind() {
    let store = temp_dir("guard-zip-staging-store");
    let archive = temp_dir("guard-zip-staging").join("liar.zip");
    write_zip_with_a_small_size(&archive, "SKILL.md", &vec![b'a'; 1024 * 1024]);
    let temporary = temp_dir("guard-zip-staging-tmp");

    let output = Command::new(BINARY)
        .env("TMPDIR", &temporary)
        .args(["catalog", "add", &path(&archive), "--store", &path(&store)])
        .output()
        .expect("run the skrun binary");

    assert_ne!(output.status.code(), Some(0));
    let left: Vec<PathBuf> = fs::read_dir(&temporary)
        .expect("read the temporary folder")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    assert!(left.is_empty(), "the staging went away: {left:?}");
}

#[test]
fn a_zip_that_inflates_its_compressed_size_is_refused() {
    let store = temp_dir("guard-zip-ratio-store");
    let archive = temp_dir("guard-zip-ratio").join("bomb.zip");
    // The entry holds four megabytes of bytes and says that it holds ten.
    // The compressed size of the entry is inflated, so a check that reads
    // the header of the archive would let the entry through.
    write_zip_with_an_inflated_size(&archive, "SKILL.md", &vec![b'a'; 4 * 1024 * 1024]);

    let output = Command::new(BINARY)
        .args(["catalog", "add", &path(&archive), "--store", &path(&store)])
        .output()
        .expect("run the skrun binary");

    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("compression ratio"), "stderr: {stderr}");
    assert!(!store.join("skills").exists());
}

#[test]
fn the_store_keeps_its_state_to_itself() {
    let source = skill_fixture("guard-permissions", &[("run.py", "import os\n")]);
    let store = temp_dir("guard-permissions-store");

    let version = add(&source, &store);

    assert_eq!(mode_of(&store), 0o700);
    assert_eq!(mode_of(&store.join("skills")), 0o700);
    assert_eq!(mode_of(&store.join("skills/demo")), 0o700);
    assert_eq!(mode_of(&store.join("records")), 0o700);
    assert_eq!(mode_of(&store.join("records/demo.json")), 0o600);
    let stored = store.join("skills/demo").join(&version.hash);
    assert_eq!(mode_of(&stored), 0o500);
    assert_eq!(mode_of(&stored.join("run.py")), 0o400);
    assert_eq!(mode_of(&stored.join("SKILL.md")), 0o400);
}

#[test]
fn an_import_of_a_call_with_a_prefix_is_an_import() {
    // A literal of python can hold a prefix, so `r"socket"` names the module
    // `socket` as well.
    let source = skill_fixture(
        "guard-import-prefix",
        &[(
            "run.py",
            "module = __import__(r\"socket\")\n\nprint(module)\n",
        )],
    );
    let store = temp_dir("guard-import-prefix-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run.py")
        .expect("a script");
    assert!(script.imports.contains(&"socket".to_string()), "{script:?}");
    assert_eq!(version.tier, "L2");
    assert!(
        version
            .tier_reasons
            .iter()
            .any(|reason| reason.contains("risky import: socket")),
        "{:?}",
        version.tier_reasons
    );
}

#[test]
fn a_bracket_that_closes_another_bracket_needs_a_container() {
    // The interpreter refuses a bracket that closes another bracket.
    let source = skill_fixture("guard-bracket-mismatch", &[("run.py", "x = ([)]\n")]);
    let store = temp_dir("guard-bracket-mismatch-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "unparsed")
        .expect("an unparsed finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.py");
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_bracket_that_is_never_closed_needs_a_container() {
    // The interpreter refuses a bracket that is never closed.
    let source = skill_fixture("guard-bracket-open", &[("run.py", "x = (1, 2\n")]);
    let store = temp_dir("guard-bracket-open-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "unparsed")
        .expect("an unparsed finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.py");
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_secret_of_a_json_file_is_never_printed() {
    let source = temp_dir("guard-secret-json").join("demo");
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The key of the json file is in quotes, so the value of the key is a
    // secret as well.
    write(
        &source.join("config.json"),
        "{\"password\": \"hunter2xx\"}\n",
    );
    let store = temp_dir("guard-secret-json-store");

    let added = Command::new(BINARY)
        .args([
            "catalog",
            "add",
            &path(&source),
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(added.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&added.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");

    let inspected = Command::new(BINARY)
        .args([
            "catalog",
            "inspect",
            "demo",
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(inspected.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&inspected.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(!record.contains("hunter2xx"), "record: {record}");

    // The secret is a finding of its own.
    let record = store::read_record(&store, "demo").expect("a record");
    let finding = record.versions[0]
        .findings
        .iter()
        .find(|finding| finding.rule == "secret")
        .expect("a secret finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "config.json");
    assert_eq!(finding.line, Some(1));
}

#[test]
fn a_zip_that_holds_more_bytes_than_it_says_is_refused() {
    let store = temp_dir("guard-zip-bytes-store");
    let archive = temp_dir("guard-zip-bytes").join("liar.zip");
    let temporary = temp_dir("guard-zip-bytes-tmp");
    // The archive holds two entries of thirty megabytes each and says that
    // every entry holds ten bytes, so only the bytes that are really
    // written tell that the archive is too big.
    write_zip_with_small_sizes(
        &archive,
        &[("one.bin", 30_000_000), ("two.bin", 30_000_000)],
    );

    let output = Command::new(BINARY)
        .env("TMPDIR", &temporary)
        .args(["catalog", "add", &path(&archive), "--store", &path(&store)])
        .output()
        .expect("run the skrun binary");

    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("more than"), "stderr: {stderr}");
    // Nothing of the refused archive is left behind.
    assert!(!store.join("skills").exists());
    assert!(!store.join("records").exists());
    if store.exists() {
        let left: Vec<PathBuf> = fs::read_dir(&store)
            .expect("read the store")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect();
        assert!(left.is_empty(), "the store holds nothing: {left:?}");
    }
    let left: Vec<PathBuf> = fs::read_dir(&temporary)
        .expect("read the temporary folder")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    assert!(left.is_empty(), "the staging went away: {left:?}");
}

#[test]
fn a_secret_of_a_json_file_with_an_escaped_quote_is_never_printed() {
    let source = temp_dir("guard-secret-json-escaped").join("demo");
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The value of the key holds an escaped quote, so the value goes on
    // behind it.
    write(
        &source.join("config.json"),
        "{\"password\":\"ab\\\"hunter2xx\"}\n",
    );
    let store = temp_dir("guard-secret-json-escaped-store");

    let added = Command::new(BINARY)
        .args([
            "catalog",
            "add",
            &path(&source),
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(added.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&added.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");

    let inspected = Command::new(BINARY)
        .args([
            "catalog",
            "inspect",
            "demo",
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(inspected.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&inspected.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(!record.contains("hunter2xx"), "record: {record}");

    // The secret is a finding of its own.
    let record = store::read_record(&store, "demo").expect("a record");
    let finding = record.versions[0]
        .findings
        .iter()
        .find(|finding| finding.rule == "secret")
        .expect("a secret finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "config.json");
    assert_eq!(finding.line, Some(1));
}

#[test]
fn a_secret_of_a_yaml_file_with_an_escaped_quote_is_never_printed() {
    let source = temp_dir("guard-secret-yaml-escaped").join("demo");
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // Two quotes in a row are an escaped quote of a yaml file, so the value
    // of the key goes on behind them.
    write(&source.join("config.yaml"), "password: 'ab''hunter2xx'\n");
    let store = temp_dir("guard-secret-yaml-escaped-store");

    let added = Command::new(BINARY)
        .args([
            "catalog",
            "add",
            &path(&source),
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(added.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&added.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");

    let inspected = Command::new(BINARY)
        .args([
            "catalog",
            "inspect",
            "demo",
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(inspected.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&inspected.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(!record.contains("hunter2xx"), "record: {record}");

    // The secret is a finding of its own.
    let record = store::read_record(&store, "demo").expect("a record");
    let finding = record.versions[0]
        .findings
        .iter()
        .find(|finding| finding.rule == "secret")
        .expect("a secret finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "config.yaml");
    assert_eq!(finding.line, Some(1));
}

#[test]
fn a_secret_of_a_json_file_with_an_escaped_key_is_never_printed() {
    let source = temp_dir("guard-secret-json-key").join("demo");
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The key of the json file hides its name behind an escape, so the key
    // is read with the escape decoded.
    write(
        &source.join("config.json"),
        "{\"pass\\u0077ord\": \"hunter2xx\"}\n",
    );
    let store = temp_dir("guard-secret-json-key-store");

    let added = Command::new(BINARY)
        .args([
            "catalog",
            "add",
            &path(&source),
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(added.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&added.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");

    let inspected = Command::new(BINARY)
        .args([
            "catalog",
            "inspect",
            "demo",
            "--store",
            &path(&store),
            "--json",
        ])
        .output()
        .expect("run the skrun binary");
    assert_eq!(inspected.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&inspected.stdout);
    assert!(!stdout.contains("hunter2xx"), "stdout: {stdout}");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(!record.contains("hunter2xx"), "record: {record}");

    // The secret is a finding of its own.
    let record = store::read_record(&store, "demo").expect("a record");
    let finding = record.versions[0]
        .findings
        .iter()
        .find(|finding| finding.rule == "secret")
        .expect("a secret finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "config.json");
    assert_eq!(finding.line, Some(1));
}

#[test]
fn a_value_with_spaces_around_it_is_a_secret() {
    let source = temp_dir("guard-placeholder-spaces").join("demo");
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The spaces of the value are part of the value, so the value is no
    // placeholder and so it is a secret.
    write(
        &source.join("config.json"),
        "{\"password\": \" changeme \"}\n",
    );
    let store = temp_dir("guard-placeholder-spaces-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "secret")
        .expect("a secret finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "config.json");
    assert_eq!(finding.line, Some(1));
}

#[test]
fn a_value_that_holds_a_closed_placeholder_is_a_secret() {
    let source = temp_dir("guard-closed-placeholder").join("demo");
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The value holds a closed placeholder and a secret behind it, so the
    // value is a secret.
    write(
        &source.join("config.json"),
        "{\"password\": \"<token>actual-secret<end>\"}\n",
    );
    let store = temp_dir("guard-closed-placeholder-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "secret")
        .expect("a secret finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "config.json");
    assert_eq!(finding.line, Some(1));
}

#[test]
fn an_import_of_a_call_with_an_octal_escape_is_an_import() {
    // An escape of one to three octal digits is an escape of python, so the
    // name of the module is read behind it.
    let source = skill_fixture(
        "guard-import-octal",
        &[(
            "run.py",
            "module = __import__(\"\\163ocket\")\n\nprint(module)\n",
        )],
    );
    let store = temp_dir("guard-import-octal-store");

    let version = add(&source, &store);

    let script = version
        .scripts
        .iter()
        .find(|script| script.path == "run.py")
        .expect("a script");
    assert!(script.imports.contains(&"socket".to_string()), "{script:?}");
    assert_eq!(version.tier, "L2");
    assert!(
        version
            .tier_reasons
            .iter()
            .any(|reason| reason.contains("risky import: socket")),
        "{:?}",
        version.tier_reasons
    );
}

#[test]
fn a_placeholder_of_a_json_file_is_no_secret() {
    let source = temp_dir("guard-placeholder-json").join("demo");
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The whole value is a placeholder, so the value is no secret.
    write(
        &source.join("config.json"),
        "{\"password\": \"changeme\"}\n",
    );
    let store = temp_dir("guard-placeholder-json-store");

    let version = add(&source, &store);

    assert!(
        !version
            .findings
            .iter()
            .any(|finding| finding.rule == "secret"),
        "{:?}",
        version.findings
    );
}

#[test]
fn a_value_that_only_holds_a_placeholder_is_a_secret() {
    let source = temp_dir("guard-partial-placeholder").join("demo");
    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    // The value holds a placeholder in the middle of it, so the value is a
    // secret.
    write(
        &source.join("config.json"),
        "{\"password\": \"abcchangeme123\"}\n",
    );
    let store = temp_dir("guard-partial-placeholder-store");

    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "secret")
        .expect("a secret finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "config.json");
    assert_eq!(finding.line, Some(1));
}

/// Add a skill to a store and return the version that was stored.
fn add(source: &Path, store: &Path) -> StoredVersion {
    store::add_skill(&AddOptions {
        source: source.to_path_buf(),
        store: Some(store.to_path_buf()),
        id: None,
        from: None,
    })
    .expect("add the skill to the store")
    .version
}

/// A skill directory with the given files.
///
/// The skill lives in a folder named `demo`, because the id of a skill comes
/// from the name of its source.
fn skill_fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = temp_dir(name).join("demo");
    write(
        &dir.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    for (file, text) in files {
        write(&dir.join(file), text);
    }
    dir
}

/// A zip archive whose entry holds more bytes than its header says.
///
/// The archive is written with the zip crate and the size of the entry is
/// patched afterwards, so the archive holds a megabyte of bytes and says
/// that it holds ten.
fn write_zip_with_a_small_size(path: &Path, name: &str, content: &[u8]) {
    if path.exists() {
        return;
    }
    let file = fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file(name, options).unwrap();
    writer.write_all(content).unwrap();
    writer.finish().unwrap();

    let mut bytes = fs::read(path).unwrap();
    patch_uncompressed_size(&mut bytes, 10);
    fs::write(path, bytes).unwrap();
}

/// The size of the entry of an archive is set to a small number.
///
/// The size of the entry sits in the header of the entry and in the
/// directory of the archive, so both are patched.
fn patch_uncompressed_size(bytes: &mut [u8], size: u32) {
    // The end of the archive holds the place of the directory of the
    // archive.
    let end = bytes.len() - 22;
    assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
    let offset = u32::from_le_bytes(
        bytes[end + 16..end + 20]
            .try_into()
            .expect("four bytes of the offset"),
    ) as usize;
    assert_eq!(&bytes[offset..offset + 4], b"PK\x01\x02");
    bytes[offset + 24..offset + 28].copy_from_slice(&size.to_le_bytes());
    // The header of the entry holds the size as well.
    assert_eq!(&bytes[0..4], b"PK\x03\x04");
    bytes[22..26].copy_from_slice(&size.to_le_bytes());
}

/// A zip archive whose entry says that it holds more compressed bytes than
/// the archive holds.
///
/// The archive is written with the zip crate and the compressed size of the
/// entry is patched afterwards, so the archive holds four megabytes of
/// bytes and says that it holds a megabyte of compressed bytes.
fn write_zip_with_an_inflated_size(path: &Path, name: &str, content: &[u8]) {
    let file = fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file(name, options).unwrap();
    writer.write_all(content).unwrap();
    writer.finish().unwrap();

    let mut bytes = fs::read(path).unwrap();
    patch_compressed_size(&mut bytes, 1024 * 1024);
    fs::write(path, bytes).unwrap();
}

/// The compressed size of the entry of an archive is set to a big number.
///
/// The size of the entry sits in the header of the entry and in the
/// directory of the archive, so both are patched.
fn patch_compressed_size(bytes: &mut [u8], size: u32) {
    // The end of the archive holds the place of the directory of the
    // archive.
    let end = bytes.len() - 22;
    assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
    let offset = u32::from_le_bytes(
        bytes[end + 16..end + 20]
            .try_into()
            .expect("four bytes of the offset"),
    ) as usize;
    assert_eq!(&bytes[offset..offset + 4], b"PK\x01\x02");
    bytes[offset + 20..offset + 24].copy_from_slice(&size.to_le_bytes());
    // The header of the entry holds the size as well.
    assert_eq!(&bytes[0..4], b"PK\x03\x04");
    bytes[18..22].copy_from_slice(&size.to_le_bytes());
}

/// A zip archive whose entries hold more bytes than their headers say.
///
/// The archive is written with the zip crate and the size of every entry is
/// patched afterwards, so the archive holds sixty megabytes of bytes and
/// says that every entry holds ten.
fn write_zip_with_small_sizes(path: &Path, entries: &[(&str, usize)]) {
    let file = fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let block = vec![b'a'; 1024 * 1024];
    for (name, size) in entries {
        writer.start_file(*name, options).unwrap();
        // The bytes of the entry are written in blocks, so the memory of the
        // test stays small.
        let mut left = *size;
        while left > 0 {
            let take = left.min(block.len());
            writer.write_all(&block[..take]).unwrap();
            left -= take;
        }
    }
    writer.finish().unwrap();

    patch_every_uncompressed_size(path, 10);
}

/// The size of every entry of an archive is set to a small number.
///
/// The size of an entry sits in the header of the entry and in the directory
/// of the archive, so both are patched.
fn patch_every_uncompressed_size(path: &Path, size: u32) {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let length = file.metadata().unwrap().len() as usize;
    // The end of the archive holds the place of the directory of the
    // archive.
    file.seek(SeekFrom::Start((length - 22) as u64)).unwrap();
    let mut end = [0u8; 22];
    file.read_exact(&mut end).unwrap();
    assert_eq!(&end[0..4], b"PK\x05\x06");
    let count = u16::from_le_bytes(end[10..12].try_into().unwrap()) as usize;
    let mut offset = u32::from_le_bytes(end[16..20].try_into().unwrap()) as usize;
    for _ in 0..count {
        file.seek(SeekFrom::Start(offset as u64)).unwrap();
        let mut header = [0u8; 46];
        file.read_exact(&mut header).unwrap();
        assert_eq!(&header[0..4], b"PK\x01\x02");
        // The name of the entry, the extra field of the entry and the
        // comment of the entry come before the data of the entry.
        let name = u16::from_le_bytes(header[28..30].try_into().unwrap()) as usize;
        let extra = u16::from_le_bytes(header[30..32].try_into().unwrap()) as usize;
        let comment = u16::from_le_bytes(header[32..34].try_into().unwrap()) as usize;
        // The header of the entry sits at the place the directory names.
        let local = u32::from_le_bytes(header[42..46].try_into().unwrap()) as usize;
        // The size of the entry sits in the directory of the archive.
        file.seek(SeekFrom::Start((offset + 24) as u64)).unwrap();
        file.write_all(&size.to_le_bytes()).unwrap();
        // The header of the entry holds the size as well.
        file.seek(SeekFrom::Start((local + 22) as u64)).unwrap();
        file.write_all(&size.to_le_bytes()).unwrap();
        offset += 46 + name + extra + comment;
    }
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
        "skrun-store-guard-test-{name}-{}-{now}-{count}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
