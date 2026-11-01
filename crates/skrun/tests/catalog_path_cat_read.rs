//! The catalog path and cat commands look at a stored skill and never run it.
//!
//! The tests run the built `skrun` binary as a subprocess, so they cover the
//! real command output: stdout and stderr together, exactly as a user sees
//! them. Every fixture is only ever copied, hashed and scanned, never run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

#[test]
fn the_path_command_shows_the_folder_of_the_newest_version() {
    let source = skill_fixture("path-newest");
    let store = temp_dir("path-newest-store");

    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&["catalog", "path", "demo", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let folder = store.join("skills").join("demo").join(hash_of(&added));
    assert_eq!(stdout.trim(), folder.to_string_lossy(), "{stdout}");
    assert!(folder.join("SKILL.md").is_file());
}

#[test]
fn the_path_command_shows_the_folder_of_a_named_version() {
    let source = skill_fixture("path-version");
    let store = temp_dir("path-version-store");

    let first = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(first.status.code(), Some(0));
    let first_hash = hash_of(&first);

    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo two\n",
    );
    let second = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(second.status.code(), Some(0));
    let second_hash = hash_of(&second);
    assert_ne!(first_hash, second_hash);

    // The newest version is the one that was added last.
    let newest = run(&["catalog", "path", "demo", "--store", &path(&store)]);
    assert_eq!(newest.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&newest.stdout);
    assert_eq!(
        stdout.trim(),
        store
            .join("skills")
            .join("demo")
            .join(&second_hash)
            .to_string_lossy(),
        "{stdout}"
    );

    // The named version is the one that was named.
    let named = run(&[
        "catalog",
        "path",
        "demo",
        "--store",
        &path(&store),
        "--version",
        &first_hash,
    ]);
    assert_eq!(named.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&named.stdout);
    assert_eq!(
        stdout.trim(),
        store
            .join("skills")
            .join("demo")
            .join(&first_hash)
            .to_string_lossy(),
        "{stdout}"
    );
}

#[test]
fn the_path_command_shows_an_absolute_path_for_a_relative_store() {
    let source = skill_fixture("path-relative");
    let root = temp_dir("path-relative-root");
    let store = root.join("store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The store is named by a path that is relative to the folder of the
    // command, so the path that is printed is absolute all the same.
    let output = Command::new(BINARY)
        .args(["catalog", "path", "demo", "--store", "store"])
        .current_dir(&root)
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let printed = Path::new(stdout.trim());
    assert!(printed.is_absolute(), "{stdout}");
    assert!(
        printed.ends_with(Path::new("skills").join("demo").join(hash_of(&added))),
        "{stdout}"
    );
    assert!(printed.join("SKILL.md").is_file());
}

#[test]
fn the_path_command_fails_for_an_unknown_id() {
    let store = temp_dir("path-unknown-store");

    let output = run(&["catalog", "path", "nothing", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn the_path_command_fails_for_an_unknown_version() {
    let source = skill_fixture("path-unknown-version");
    let store = temp_dir("path-unknown-version-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "path",
        "demo",
        "--store",
        &path(&store),
        "--version",
        "0123456789abcdef",
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn the_cat_command_shows_the_file_of_the_skill() {
    let source = skill_fixture("cat-default");
    let store = temp_dir("cat-default-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&["catalog", "cat", "demo", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, SKILL_TEXT);
    let stored = store.join("skills").join("demo").join(hash_of(&added));
    assert_eq!(stdout, fs::read_to_string(stored.join("SKILL.md")).unwrap());
}

#[test]
fn the_cat_command_shows_a_named_file() {
    let source = skill_fixture("cat-file");
    let store = temp_dir("cat-file-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "docs/guide.md",
        "--store",
        &path(&store),
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, GUIDE_TEXT);
    let stored = store.join("skills").join("demo").join(hash_of(&added));
    assert_eq!(
        stdout,
        fs::read_to_string(stored.join("docs/guide.md")).unwrap()
    );
}

#[test]
fn the_cat_command_shows_the_file_of_a_named_version() {
    let source = skill_fixture("cat-version");
    let store = temp_dir("cat-version-store");

    let first = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(first.status.code(), Some(0));
    let first_hash = hash_of(&first);

    write(
        &source.join("SKILL.md"),
        "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo two\n",
    );
    let second = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(second.status.code(), Some(0));

    // The newest version is shown when no version is named.
    let newest = run(&["catalog", "cat", "demo", "--store", &path(&store)]);
    assert_eq!(newest.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&newest.stdout).contains("# Demo two"));

    // The named version is the one that was named.
    let named = run(&[
        "catalog",
        "cat",
        "demo",
        "--store",
        &path(&store),
        "--version",
        &first_hash,
    ]);
    assert_eq!(named.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&named.stdout);
    assert_eq!(stdout, SKILL_TEXT);
}

#[test]
fn the_cat_command_refuses_a_path_that_leaves_the_version() {
    let source = skill_fixture("cat-traversal");
    let store = temp_dir("cat-traversal-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "../../../etc/passwd",
        "--store",
        &path(&store),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("must not leave"), "{stderr}");
}

#[test]
fn the_cat_command_refuses_an_absolute_path() {
    let source = skill_fixture("cat-absolute");
    let store = temp_dir("cat-absolute-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "/etc/passwd",
        "--store",
        &path(&store),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("inside the stored version"), "{stderr}");
}

#[test]
fn the_cat_command_refuses_a_symlink() {
    let source = skill_fixture("cat-symlink");
    let store = temp_dir("cat-symlink-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The copy in the store is read only, so the folder is opened before the
    // link is put into it.
    let version = store.join("skills").join("demo").join(hash_of(&added));
    make_writable(&version);
    std::os::unix::fs::symlink("/etc/passwd", version.join("link.md")).unwrap();

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "link.md",
        "--store",
        &path(&store),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("symlink"), "{stderr}");
}

#[test]
fn the_cat_command_refuses_a_file_that_is_not_text() {
    let source = skill_fixture("cat-binary");
    let store = temp_dir("cat-binary-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let version = store.join("skills").join("demo").join(hash_of(&added));
    make_writable(&version);
    fs::write(version.join("raw.md"), [0xff, 0xfe, 0x00, 0x01]).unwrap();

    let output = run(&["catalog", "cat", "demo", "raw.md", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("UTF-8"), "{stderr}");
}

#[test]
fn the_cat_command_refuses_a_file_that_is_too_big() {
    let source = skill_fixture("cat-big");
    let store = temp_dir("cat-big-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let version = store.join("skills").join("demo").join(hash_of(&added));
    make_writable(&version);
    fs::write(version.join("big.md"), vec![b'x'; 2 * 1000 * 1000 + 1]).unwrap();

    let output = run(&["catalog", "cat", "demo", "big.md", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bigger than"), "{stderr}");
}

#[test]
fn the_list_command_shows_the_markdown_files() {
    let source = skill_fixture("cat-list");
    let store = temp_dir("cat-list-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&["catalog", "cat", "demo", "--list", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "SKILL.md\ndocs/guide.md\n", "{stdout}");
}

#[test]
fn the_require_safe_flag_refuses_a_version_that_is_not_safe() {
    let source = skill_fixture("cat-unsafe");
    fs::write(source.join("tool.bin"), b"\x7fELF\x02\x01\x01\x00rest").unwrap();
    let store = temp_dir("cat-unsafe-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&added.stdout);
    assert!(stdout.contains("tier L3"), "{stdout}");

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "--store",
        &path(&store),
        "--require-safe",
    ]);

    assert_eq!(output.status.code(), Some(3));
    // Nothing of the file reaches the output.
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("L3"), "{stderr}");
    assert!(stderr.contains("not safe"), "{stderr}");
}

#[test]
fn the_require_safe_flag_reads_the_version_that_was_named() {
    let source = skill_fixture("cat-safe-version");
    let store = temp_dir("cat-safe-version-store");

    // The first version holds no binary, so it is safe.
    let first = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(first.status.code(), Some(0));
    let first_hash = hash_of(&first);

    // The second version holds a binary, so it is of the tier L3.
    fs::write(source.join("tool.bin"), b"\x7fELF\x02\x01\x01\x00rest").unwrap();
    let second = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(second.status.code(), Some(0));
    assert_ne!(first_hash, hash_of(&second));

    // The newest version is not safe, so it is refused and nothing of it
    // reaches the output.
    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "--store",
        &path(&store),
        "--require-safe",
    ]);
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());

    // The named version is safe, so it is shown.
    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "--store",
        &path(&store),
        "--version",
        &first_hash,
        "--require-safe",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), SKILL_TEXT);
}

#[test]
fn the_require_safe_flag_shows_a_version_that_is_safe() {
    let source = skill_fixture("cat-safe");
    let store = temp_dir("cat-safe-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "--store",
        &path(&store),
        "--require-safe",
    ]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, SKILL_TEXT);
}

#[test]
fn an_unknown_id_fails_with_status_one() {
    let store = temp_dir("cat-unknown-store");

    let output = run(&["catalog", "cat", "nothing", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn the_path_command_fails_for_a_version_whose_folder_is_gone() {
    let source = skill_fixture("path-gone");
    let store = temp_dir("path-gone-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The copy in the store is read only, so the folder is opened before
    // it is removed.
    let version = store.join("skills").join("demo").join(hash_of(&added));
    make_writable(&version);
    fs::remove_dir_all(&version).unwrap();

    let output = run(&["catalog", "path", "demo", "--store", &path(&store)]);

    // The folder of the version is gone, so the path of it is not shown.
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn the_path_command_fails_for_a_version_whose_folder_is_a_link() {
    let source = skill_fixture("path-link");
    let store = temp_dir("path-link-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The copy in the store is read only, so the folder is opened before
    // the link is put in the place of the folder of the version.
    let version = store.join("skills").join("demo").join(hash_of(&added));
    make_writable(&version);
    let outside = temp_dir("path-link-outside");
    fs::remove_dir_all(&version).unwrap();
    std::os::unix::fs::symlink(&outside, &version).unwrap();

    let output = run(&["catalog", "path", "demo", "--store", &path(&store)]);

    // The folder of the version is a link, so it is not a folder of the
    // store and its path is not shown.
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn an_empty_version_names_no_version() {
    let source = skill_fixture("empty-version");
    let store = temp_dir("empty-version-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // A version that is named but holds no name is not the newest
    // version, so the command fails.
    let output = run(&[
        "catalog",
        "path",
        "demo",
        "--store",
        &path(&store),
        "--version",
        "",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "--store",
        &path(&store),
        "--version",
        "",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn the_cat_command_refuses_a_folder_that_is_a_link() {
    let source = skill_fixture("cat-link-folder");
    let store = temp_dir("cat-link-folder-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The copy in the store is read only, so the folder is opened before
    // the link is put in the place of the folder of the guide.
    let version = store.join("skills").join("demo").join(hash_of(&added));
    make_writable(&version);
    let outside = temp_dir("cat-link-folder-outside");
    write(&outside.join("secret.md"), "# Secret\n");
    fs::remove_dir_all(version.join("docs")).unwrap();
    std::os::unix::fs::symlink(&outside, version.join("docs")).unwrap();

    let output = run(&[
        "catalog",
        "cat",
        "demo",
        "docs/secret.md",
        "--store",
        &path(&store),
    ]);

    // The folder is a link, so the file behind it is never read.
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("symlink"), "{stderr}");
    assert!(!stderr.contains("Secret"), "{stderr}");
}

#[test]
fn the_cat_command_refuses_a_skill_folder_that_is_a_link() {
    let source = skill_fixture("cat-link-skill");
    let store = temp_dir("cat-link-skill-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The folder of the skill is put in the place of a link that points at
    // a folder that holds the very same version, so the file behind the
    // link is not a file of the store.
    let outside = temp_dir("cat-link-skill-outside");
    let hash = hash_of(&added);
    write(&outside.join(&hash).join("SKILL.md"), "# Secret\n");
    let skills = store.join("skills");
    make_writable(&skills.join("demo"));
    fs::remove_dir_all(skills.join("demo")).unwrap();
    std::os::unix::fs::symlink(&outside, skills.join("demo")).unwrap();

    let output = run(&["catalog", "cat", "demo", "--store", &path(&store)]);

    // The folder of the skill is a link, so the file behind it is never
    // read.
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("symlink"), "{stderr}");
    assert!(!stderr.contains("Secret"), "{stderr}");
}

#[test]
fn the_list_command_refuses_a_skill_folder_that_is_a_link() {
    let source = skill_fixture("list-link-skill");
    let store = temp_dir("list-link-skill-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The folder of the skill is put in the place of a link that points at
    // a folder that holds the very same version, so the files behind the
    // link are not files of the store.
    let outside = temp_dir("list-link-skill-outside");
    let hash = hash_of(&added);
    write(&outside.join(&hash).join("secret.md"), "# Secret\n");
    let skills = store.join("skills");
    make_writable(&skills.join("demo"));
    fs::remove_dir_all(skills.join("demo")).unwrap();
    std::os::unix::fs::symlink(&outside, skills.join("demo")).unwrap();

    let output = run(&["catalog", "cat", "demo", "--list", "--store", &path(&store)]);

    // The folder of the skill is a link, so the files behind it are never
    // listed.
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("symlink"), "{stderr}");
    assert!(!stderr.contains("Secret"), "{stderr}");
}

#[test]
fn the_list_command_skips_a_folder_that_is_a_link() {
    let source = skill_fixture("list-link-folder");
    let store = temp_dir("list-link-folder-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The folder of the guide is put in the place of a link, so the file
    // behind the link is not a file of the stored version.
    let version = store.join("skills").join("demo").join(hash_of(&added));
    make_writable(&version);
    let outside = temp_dir("list-link-folder-outside");
    write(&outside.join("secret.md"), "# Secret\n");
    fs::remove_dir_all(version.join("docs")).unwrap();
    std::os::unix::fs::symlink(&outside, version.join("docs")).unwrap();

    let output = run(&["catalog", "cat", "demo", "--list", "--store", &path(&store)]);

    // The folder of the guide is a link, so it is never read and the file
    // behind it is never listed.
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "SKILL.md\n", "{stdout}");
}

#[test]
fn the_error_of_a_broken_record_hides_a_secret() {
    let source = skill_fixture("broken-record");
    let store = temp_dir("broken-record-store");
    let added = run(&["catalog", "add", &path(&source), "--store", &path(&store)]);
    assert_eq!(added.status.code(), Some(0));

    // The record is broken, so it cannot be read, and the error of the
    // record holds the value of the record.
    let record = store.join("records").join("demo.json");
    write(
        &record,
        "{\"schema_version\":\"AKIAIOSFODNN7EXAMPLE\",\"id\":\"demo\",\"versions\":[]}",
    );

    let output = run(&["catalog", "path", "demo", "--store", &path(&store)]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("AKIAIOSFODNN7EXAMPLE"), "stderr: {stderr}");
}

/// The text of the file of the skill of the fixture.
const SKILL_TEXT: &str = "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n";

/// The text of the guide of the fixture.
const GUIDE_TEXT: &str = "# Guide\n\nA guide of the demo skill.\n";

/// Run the binary with the given arguments.
fn run(args: &[&str]) -> Output {
    Command::new(BINARY)
        .args(args)
        .output()
        .expect("run the skrun binary")
}

/// The hash of the version that an add stored.
///
/// The hash is the third word of the line of an add.
fn hash_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(2)
        .expect("the hash of the stored version")
        .to_string()
}

/// A skill directory with a file of markdown, a guide and a script.
///
/// The skill lives in a folder named `demo`, because the id of a skill comes
/// from the name of its source.
fn skill_fixture(name: &str) -> PathBuf {
    let dir = temp_dir(name).join("demo");
    write(&dir.join("SKILL.md"), SKILL_TEXT);
    write(&dir.join("docs/guide.md"), GUIDE_TEXT);
    write(&dir.join("run.py"), "print('hello')\n");
    dir
}

/// Make a folder of the store writable again.
///
/// The copy in the store is read only, so a test that puts a file into it
/// has to open the folder first. Every folder below it is opened as well,
/// so a test that removes a file of the copy can do it.
fn make_writable(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(dir).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(dir, permissions).unwrap();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() && !path.is_symlink() {
            make_writable(&path);
        }
    }
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
    let root = std::env::temp_dir().join(format!(
        "skrun-path-cat-test-{name}-{}-{now}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
