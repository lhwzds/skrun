//! A skill of a repository of GitHub is fetched, stored and never run.
//!
//! The tests never reach the network: the git command line is a trait, so a
//! test holds a repository of its own, and the command line of the machine
//! is a program of the `SKRUN_GIT` variable, so a test can hold a fake git
//! that writes a repository of its own.

use skrun::store::{self, GitHubAddOptions, GitOutput, GitRunner};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// The `skrun` binary that cargo built for this test run.
const BINARY: &str = env!("CARGO_BIN_EXE_skrun");

/// The commit that the fake git reports.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// Another commit that the fake git reports.
const OTHER_SHA: &str = "89abcdef0123456789abcdef0123456789abcdef";

/// The skill of the fake repository.
const SKILL: &str = "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n";

/// The script of the fake repository.
///
/// The script holds a key shaped value, so the scan of the store finds it
/// and the tier of the skill is the tier of a script that touches the file
/// system.
const SCRIPT: &str = "import os\n\nKEY = \"AKIAIOSFODNN7EXAMPLE\"\nprint(KEY)\n";

#[test]
fn a_source_of_github_is_read() {
    let source = store::parse_github_source("github:owner/repo").unwrap();

    assert_eq!(source.owner, "owner");
    assert_eq!(source.repo, "repo");
    assert_eq!(source.reference, None);
    assert_eq!(source.subpath, None);
}

#[test]
fn a_source_of_github_with_a_ref_is_read() {
    let source = store::parse_github_source("github:owner/repo@v1.2.3").unwrap();

    assert_eq!(source.owner, "owner");
    assert_eq!(source.repo, "repo");
    assert_eq!(source.reference, Some("v1.2.3".to_string()));
    assert_eq!(source.subpath, None);
}

#[test]
fn a_source_of_github_with_a_subpath_is_read() {
    let source = store::parse_github_source("github:owner/repo//skills/demo").unwrap();

    assert_eq!(source.owner, "owner");
    assert_eq!(source.repo, "repo");
    assert_eq!(source.reference, None);
    assert_eq!(source.subpath, Some("skills/demo".to_string()));
}

#[test]
fn a_source_of_github_with_a_ref_and_a_subpath_is_read() {
    let source = store::parse_github_source("github:owner/repo@main//skills/demo").unwrap();

    assert_eq!(source.owner, "owner");
    assert_eq!(source.repo, "repo");
    assert_eq!(source.reference, Some("main".to_string()));
    assert_eq!(source.subpath, Some("skills/demo".to_string()));
}

#[test]
fn a_source_of_github_that_is_not_a_source_is_refused() {
    let error = store::parse_github_source("owner/repo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("is not a source of GitHub"), "{printed}");
}

#[test]
fn a_source_of_github_without_a_repository_is_refused() {
    let error = store::parse_github_source("github:owner").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("github:<owner>/<repo>"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_bad_owner_is_refused() {
    let error = store::parse_github_source("github:own er/repo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("owner"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_bad_repository_is_refused() {
    let error = store::parse_github_source("github:owner/repo?x=1").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("repository"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_dot_dot_owner_is_refused() {
    let error = store::parse_github_source("github:../repo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("owner"), "{printed}");
}

#[test]
fn a_source_of_github_with_an_empty_ref_is_refused() {
    let error = store::parse_github_source("github:owner/repo@").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("ref"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_dot_dot_ref_is_refused() {
    let error = store::parse_github_source("github:owner/repo@main..dev").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("`..`"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_ref_that_is_an_option_is_refused() {
    let error = store::parse_github_source("github:owner/repo@--upload-pack=touch").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("dash"), "{printed}");
}

#[test]
fn a_source_of_github_with_an_empty_subpath_is_refused() {
    let error = store::parse_github_source("github:owner/repo//").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("subpath"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_subpath_that_leaves_the_repository_is_refused() {
    let error = store::parse_github_source("github:owner/repo//skills/../../etc").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("`..`"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_subpath_that_is_absolute_is_refused() {
    let error = store::parse_github_source("github:owner/repo///etc").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("slash"), "{printed}");
}

#[test]
fn a_source_of_github_with_an_owner_that_starts_with_a_dash_is_refused() {
    let error = store::parse_github_source("github:-owner/repo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("owner"), "{printed}");
    assert!(printed.contains("letter or a number"), "{printed}");
}

#[test]
fn a_source_of_github_with_an_owner_that_starts_with_a_dot_is_refused() {
    let error = store::parse_github_source("github:.owner/repo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("owner"), "{printed}");
    assert!(printed.contains("letter or a number"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_repository_that_starts_with_a_dash_is_refused() {
    let error = store::parse_github_source("github:owner/-repo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("repository"), "{printed}");
    assert!(printed.contains("letter or a number"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_repository_that_ends_with_git_is_refused() {
    // The address of the fetch ends with `.git`, so a repository that ends
    // with `.git` would end up with `.git` twice.
    let error = store::parse_github_source("github:owner/repo.git").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("repository"), "{printed}");
    assert!(printed.contains("`.git`"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_ref_that_holds_a_character_that_is_not_allowed_is_refused() {
    let error = store::parse_github_source("github:owner/repo@main;rm").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("ref"), "{printed}");
    assert!(printed.contains("not allowed"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_ref_that_holds_an_underscore_is_accepted() {
    let source = store::parse_github_source("github:owner/repo@my_branch").unwrap();

    assert_eq!(source.reference.as_deref(), Some("my_branch"));
}

#[test]
fn a_source_of_github_with_a_subpath_part_that_starts_with_a_dash_is_refused() {
    let error = store::parse_github_source("github:owner/repo//skills/-demo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("subpath"), "{printed}");
    assert!(printed.contains("dash"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_subpath_part_that_is_the_current_folder_is_refused() {
    let error = store::parse_github_source("github:owner/repo//skills/./demo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("subpath"), "{printed}");
    assert!(printed.contains("`.`"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_subpath_part_that_holds_a_character_that_is_not_allowed_is_refused() {
    let error = store::parse_github_source("github:owner/repo//skills/de mo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("subpath"), "{printed}");
    assert!(printed.contains("not allowed"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_subpath_that_holds_an_empty_part_is_refused() {
    let error = store::parse_github_source("github:owner/repo//skills//demo").unwrap_err();

    let printed = format!("{error:#}");
    assert!(printed.contains("subpath"), "{printed}");
    assert!(printed.contains("inside the repository"), "{printed}");
}

#[test]
fn a_source_of_github_with_a_subpath_part_that_holds_an_underscore_is_read() {
    let source = store::parse_github_source("github:owner/repo//skills/my_demo").unwrap();

    assert_eq!(source.subpath, Some("skills/my_demo".to_string()));
}

#[test]
fn the_repository_is_fetched_with_the_git_command_line() {
    let git = FakeGit::new();
    let store = temp_dir("github-fetch-store");

    let _ = add(&git, &store, "github:owner/repo@main//skills/demo");

    let calls = git.calls();
    assert_eq!(calls.len(), 4, "{calls:?}");
    assert_eq!(calls[0], safe(&["init"]));
    // The address of the fetch is the address of the repository of GitHub,
    // and nothing else.
    assert_eq!(
        calls[1],
        safe(&[
            "fetch",
            "--depth",
            "1",
            "--no-recurse-submodules",
            "https://github.com/owner/repo.git",
            "main",
        ])
    );
    assert_eq!(calls[2], safe(&["checkout", "--detach", "FETCH_HEAD"]));
    assert_eq!(calls[3], safe(&["rev-parse", "HEAD"]));
}

#[test]
fn the_address_of_the_fetch_is_the_address_of_the_repository() {
    let git = FakeGit::new();
    let store = temp_dir("github-address-store");

    let _ = add(&git, &store, "github:owner/repo@main//skills/demo");

    let calls = git.calls();
    let address = calls[1]
        .iter()
        .find(|argument| argument.starts_with("https://"))
        .expect("the address of the fetch");
    assert_eq!(address, "https://github.com/owner/repo.git", "{calls:?}");
    // The address of the fetch is never the address of something else.
    assert!(
        !calls[1].iter().any(|argument| argument.contains("evil")),
        "{calls:?}"
    );
}

#[test]
fn the_command_line_of_git_holds_back_every_feature_that_runs_code() {
    let git = FakeGit::new();
    let store = temp_dir("github-safe-store");

    let _ = add(&git, &store, "github:owner/repo@main//skills/demo");

    let calls = git.calls();
    for call in &calls {
        let line = call.join(" ");
        assert!(line.contains("core.hooksPath=/dev/null"), "{line}");
        assert!(line.contains("credential.helper="), "{line}");
        assert!(line.contains("submodule.recurse=false"), "{line}");
        assert!(line.contains("fetch.recurseSubmodules=false"), "{line}");
        assert!(line.contains("filter.lfs.smudge="), "{line}");
        assert!(line.contains("filter.lfs.process="), "{line}");
        assert!(line.contains("filter.lfs.required=false"), "{line}");
    }
}

#[test]
fn a_repository_without_a_ref_fetches_the_default_branch() {
    let git = FakeGit::new();
    let store = temp_dir("github-default-store");

    let _ = add(&git, &store, "github:owner/repo");

    let calls = git.calls();
    assert_eq!(calls[1].last().unwrap(), "HEAD", "{calls:?}");
}

#[test]
fn the_provenance_of_a_skill_names_the_commit_that_was_fetched() {
    let git = FakeGit::new();
    let store = temp_dir("github-provenance-store");

    let result = add(&git, &store, "github:owner/repo@main//skills/demo");

    assert_eq!(
        result.version.from,
        Some(format!("github:owner/repo@{SHA}//skills/demo"))
    );
    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(
        record.contains(&format!("github:owner/repo@{SHA}//skills/demo")),
        "{record}"
    );
}

#[test]
fn the_provenance_of_a_skill_without_a_subpath_names_the_repository() {
    let git = FakeGit::new();
    let store = temp_dir("github-provenance-root-store");

    let result = add(&git, &store, "github:owner/repo");

    assert_eq!(
        result.version.from,
        Some(format!("github:owner/repo@{SHA}//"))
    );
}

#[test]
fn a_source_of_github_refuses_the_provenance_of_the_user() {
    let git = FakeGit::new();
    let store = temp_dir("github-from-store");

    let error = add_with_from(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        Some("https://example.com/repo"),
    )
    .expect_err("the provenance of the user is not allowed");

    let printed = format!("{error:#}");
    assert!(printed.contains("`--from`"), "{printed}");
    // The repository is never fetched, because the command is refused
    // before anything happens.
    assert!(git.calls().is_empty(), "{:?}", git.calls());
    assert!(!store.join("skills/demo").exists(), "the skill was stored");
}

#[test]
fn the_command_line_refuses_the_provenance_of_the_user_for_a_source_of_github() {
    let root = temp_dir("github-from-cli");
    let store = root.join("store");

    let output = Command::new(BINARY)
        .arg("catalog")
        .arg("add")
        .arg("github:owner/repo@main//skills/demo")
        .arg("--store")
        .arg(&store)
        .arg("--from")
        .arg("https://example.com/repo")
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`--from`"), "{stderr}");
    assert!(!store.join("skills/demo").exists(), "the skill was stored");
}

#[test]
fn the_id_of_a_skill_is_the_name_of_its_subpath() {
    let git = FakeGit::new();
    let store = temp_dir("github-id-store");

    let result = add(&git, &store, "github:owner/repo@main//skills/demo");

    assert_eq!(result.id, "demo");
    assert!(store.join("skills/demo").is_dir());
}

#[test]
fn the_id_of_a_skill_without_a_subpath_is_the_name_of_the_repository() {
    let git = FakeGit::new();
    let store = temp_dir("github-id-root-store");

    let result = add(&git, &store, "github:owner/repo");

    assert_eq!(result.id, "repo");
    assert!(store.join("skills/repo").is_dir());
}

#[test]
fn the_id_of_the_user_wins_over_the_id_of_the_repository() {
    let git = FakeGit::new();
    let store = temp_dir("github-id-user-store");

    let result = add_with_id(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        Some("other"),
    );

    assert_eq!(result.id, "other");
    assert!(store.join("skills/other").is_dir());
}

#[test]
fn a_skill_of_a_repository_is_scanned_like_a_skill_of_the_machine() {
    let git = FakeGit::new();
    let store = temp_dir("github-scan-store");

    let result = add(&git, &store, "github:owner/repo@main//skills/demo");

    assert_eq!(result.version.files, 2);
    let finding = result
        .version
        .findings
        .iter()
        .find(|finding| finding.rule == "secret")
        .expect("a secret finding");
    assert_eq!(finding.path, "run.py");
    assert_eq!(result.version.tier, "L1");
}

#[test]
fn the_add_of_a_source_of_github_fetches_the_repository() {
    let git = FakeGit::new();
    let store = temp_dir("github-add-store");

    let result = store::add_skill_with(
        &store::AddOptions {
            source: PathBuf::from("github:owner/repo@main//skills/demo"),
            store: Some(store.clone()),
            id: None,
            from: None,
        },
        &git,
    )
    .expect("add the skill of the repository to the store");

    assert_eq!(result.id, "demo");
    assert_eq!(
        result.version.from,
        Some(format!("github:owner/repo@{SHA}//skills/demo"))
    );
}

#[test]
fn a_repository_with_a_name_that_is_no_id_is_refused() {
    let git = FakeGit::new();
    let store = temp_dir("github-bad-id-store");

    let error = add_with(&git, &store, "github:owner/my.repo", None, None)
        .expect_err("the name of the repository is no id");

    let printed = format!("{error:#}");
    assert!(printed.contains("use `--id`"), "{printed}");
    // The name of the repository is no id, so the repository is never
    // fetched.
    assert!(git.calls().is_empty(), "{:?}", git.calls());
}

#[test]
fn a_repository_without_a_skill_is_refused() {
    let git = FakeGit::new().without_skill();
    let store = temp_dir("github-no-skill-store");

    let error = add_with(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        None,
        None,
    )
    .expect_err("the repository holds no skill");

    let printed = format!("{error:#}");
    assert!(printed.contains("SKILL.md"), "{printed}");
}

#[test]
fn a_repository_that_cannot_be_fetched_is_refused() {
    let git = FakeGit::new().failing("fetch");
    let store = temp_dir("github-fetch-fails-store");

    let error = add_with(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        None,
        None,
    )
    .expect_err("the repository cannot be fetched");

    let printed = format!("{error:#}");
    assert!(printed.contains("fetch"), "{printed}");
    assert!(printed.contains("the fetch failed"), "{printed}");
}

#[test]
fn the_temporary_directory_of_a_fetch_goes_away_when_the_add_ends() {
    let git = FakeGit::new();
    let store = temp_dir("github-cleanup-store");

    let _ = add(&git, &store, "github:owner/repo@main//skills/demo");

    let directory = git.directory();
    assert!(
        !directory.exists(),
        "the temporary directory is still there"
    );
}

#[test]
fn the_temporary_directory_of_a_fetch_goes_away_when_the_add_fails() {
    let git = FakeGit::new().without_skill();
    let store = temp_dir("github-cleanup-fail-store");

    let _ = add_with(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        None,
        None,
    );

    let directory = git.directory();
    assert!(
        !directory.exists(),
        "the temporary directory is still there"
    );
}

#[test]
fn the_temporary_directory_of_a_fetch_goes_away_when_the_fetch_fails() {
    let git = FakeGit::new().failing("fetch");
    let store = temp_dir("github-cleanup-fetch-store");

    let _ = add_with(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        None,
        None,
    );

    let directory = git.directory();
    assert!(
        !directory.exists(),
        "the temporary directory is still there"
    );
}

#[test]
fn a_repository_that_reports_an_abbreviation_of_a_commit_is_refused() {
    // The commit of the record must name the commit itself, so an
    // abbreviation of a commit is never enough.
    let git = FakeGit::new().with_sha("0123456");
    let store = temp_dir("github-short-sha-store");

    let error = add_with(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        None,
        None,
    )
    .expect_err("the commit is not a full commit id");

    let printed = format!("{error:#}");
    assert!(printed.contains("full commit id"), "{printed}");
    // Nothing of the add that failed reaches the store.
    assert!(!store.join("skills/demo").exists(), "the skill was stored");
}

#[test]
fn a_repository_that_reports_a_commit_in_upper_case_is_refused() {
    let git = FakeGit::new().with_sha("0123456789ABCDEF0123456789ABCDEF01234567");
    let store = temp_dir("github-upper-sha-store");

    let error = add_with(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        None,
        None,
    )
    .expect_err("the commit is not a commit id of git");

    let printed = format!("{error:#}");
    assert!(printed.contains("full commit id"), "{printed}");
    assert!(!store.join("skills/demo").exists(), "the skill was stored");
}

#[test]
fn a_repository_that_reports_a_commit_of_sha256_is_stored() {
    let sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let git = FakeGit::new().with_sha(sha);
    let store = temp_dir("github-sha256-store");

    let result = add(&git, &store, "github:owner/repo@main//skills/demo");

    assert_eq!(
        result.version.from,
        Some(format!("github:owner/repo@{sha}//skills/demo"))
    );
}

#[test]
fn the_commit_of_the_record_is_the_commit_that_was_fetched() {
    let git = FakeGit::new();
    let store = temp_dir("github-record-sha-store");

    let result = add(&git, &store, "github:owner/repo@main//skills/demo");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(record.contains(SHA), "{record}");
    assert!(
        record.contains(&format!("github:owner/repo@{SHA}//skills/demo")),
        "{record}"
    );
    assert_eq!(result.version.full_hash.len(), 64, "{:?}", result.version);
}

#[test]
fn a_record_that_cannot_be_written_leaves_nothing_in_the_store() {
    let git = FakeGit::new();
    let store = temp_dir("github-record-fails-store");
    // The record of the skill is a folder, so the record can never be
    // written.
    fs::create_dir_all(store.join("records/demo.json")).unwrap();

    let error = add_with(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        None,
        None,
    )
    .expect_err("the record cannot be written");

    let printed = format!("{error:#}");
    assert!(printed.contains("record"), "{printed}");
    // The store holds nothing of the add that failed: the folder of the
    // version and the folder of the skill are gone.
    assert!(!store.join("skills/demo").exists(), "the skill was stored");
    assert!(store_skills_are_empty(&store), "the store holds a skill");
}

#[test]
fn a_record_that_cannot_be_read_leaves_nothing_in_the_store() {
    let git = FakeGit::new();
    let store = temp_dir("github-record-broken-store");
    // The record of the skill holds something that is not a record.
    fs::create_dir_all(store.join("records")).unwrap();
    fs::write(store.join("records/demo.json"), "{ not a record").unwrap();

    let error = add_with(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        None,
        None,
    )
    .expect_err("the record cannot be read");

    let printed = format!("{error:#}");
    assert!(printed.contains("record"), "{printed}");
    // The store holds nothing of the add that failed.
    assert!(!store.join("skills/demo").exists(), "the skill was stored");
    assert!(store_skills_are_empty(&store), "the store holds a skill");
}

#[test]
fn the_command_line_adds_a_repository_of_github() {
    let root = temp_dir("github-cli");
    let git = fake_git(&root);
    let store = root.join("store");

    let output = Command::new(BINARY)
        .arg("catalog")
        .arg("add")
        .arg("github:owner/repo@main//skills/demo")
        .arg("--store")
        .arg(&store)
        .env("SKRUN_GIT", &git)
        .env("SKRUN_TEST_GIT_LOG", root.join("git.log"))
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("stored demo "), "{stdout}");

    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(
        record.contains(&format!("github:owner/repo@{SHA}//skills/demo")),
        "{record}"
    );
    // The key of the script of the repository never reaches the record.
    assert!(!record.contains("AKIAIOSFODNN7EXAMPLE"), "{record}");

    let log = fs::read_to_string(root.join("git.log")).unwrap();
    assert!(
        log.contains(
            "fetch --depth 1 --no-recurse-submodules https://github.com/owner/repo.git main"
        ),
        "{log}"
    );
    assert!(log.contains("checkout --detach FETCH_HEAD"), "{log}");
    assert!(log.contains("rev-parse HEAD"), "{log}");
}

#[test]
fn the_command_line_refuses_a_source_of_github_that_is_bad() {
    let output = Command::new(BINARY)
        .arg("catalog")
        .arg("add")
        .arg("github:owner/repo//skills/../../etc")
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid source of GitHub"), "{stderr}");
    assert!(stderr.contains("`..`"), "{stderr}");
}

#[test]
fn a_symlink_in_the_subpath_of_a_repository_is_refused() {
    // The folder of the skill of the repository is behind a symlink, so
    // the files of the skill come from outside the checkout.
    let outside = temp_dir("github-symlink-outside");
    fs::create_dir_all(outside.join("demo")).unwrap();
    fs::write(outside.join("demo/SKILL.md"), SKILL).unwrap();
    let git = FakeGit::new().with_link("skills", &outside);
    let store = temp_dir("github-symlink-store");

    let error = add_with(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        None,
        None,
    )
    .expect_err("the subpath of the repository holds a symlink");

    let printed = format!("{error:#}");
    assert!(printed.contains("symlink"), "{printed}");
    // The files behind the symlink never reach the store.
    assert!(!store.join("skills").exists(), "the skill was stored");
}

#[test]
fn the_provenance_of_a_skill_that_is_stored_again_names_the_commit_of_today() {
    let store = temp_dir("github-provenance-again-store");
    let first = FakeGit::new().with_sha(SHA);
    let second = FakeGit::new().with_sha(OTHER_SHA);

    let _ = add(&first, &store, "github:owner/repo@main//skills/demo");
    // The very same content comes from another commit of the repository.
    let result = add(&second, &store, "github:owner/repo@main//skills/demo");

    assert!(result.already_stored);
    assert_eq!(
        result.version.from,
        Some(format!("github:owner/repo@{OTHER_SHA}//skills/demo"))
    );
    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(
        record.contains(&format!("github:owner/repo@{OTHER_SHA}//skills/demo")),
        "{record}"
    );
    assert!(
        !record.contains(&format!("github:owner/repo@{SHA}//skills/demo")),
        "{record}"
    );
}

#[test]
fn the_provenance_of_a_source_of_github_is_always_the_repository() {
    let git = FakeGit::new();
    let store = temp_dir("github-from-again-store");

    let _ = add(&git, &store, "github:owner/repo@main//skills/demo");
    let error = add_with_from(
        &git,
        &store,
        "github:owner/repo@main//skills/demo",
        Some("https://example.com/repo"),
    )
    .expect_err("the provenance of the user is not allowed");

    let printed = format!("{error:#}");
    assert!(printed.contains("`--from`"), "{printed}");
    // The record of the skill still names the repository and the commit.
    let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
    assert!(
        record.contains(&format!("github:owner/repo@{SHA}//skills/demo")),
        "{record}"
    );
}

#[test]
fn the_hooks_of_a_repository_are_never_run() {
    let root = temp_dir("github-hooks");
    let git = fake_git(&root);
    let store = root.join("store");

    // The fake git of the test fails the command when the hooks of the
    // repository are not off.
    let output = Command::new(BINARY)
        .arg("catalog")
        .arg("add")
        .arg("github:owner/repo@main//skills/demo")
        .arg("--store")
        .arg(&store)
        .env("SKRUN_GIT", &git)
        .env("SKRUN_TEST_GIT_LOG", root.join("git.log"))
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let log = fs::read_to_string(root.join("git.log")).unwrap();
    assert!(log.contains("core.hooksPath=/dev/null"), "{log}");
    assert!(log.contains("checkout --detach FETCH_HEAD"), "{log}");
}

#[test]
fn the_git_command_line_never_asks_for_a_password() {
    let root = temp_dir("github-credentials");
    let git = fake_git(&root);
    let store = root.join("store");

    // The environment of the shell holds prompt programs of git and of
    // ssh, and a credential manager that likes to ask.
    let output = Command::new(BINARY)
        .arg("catalog")
        .arg("add")
        .arg("github:owner/repo@main//skills/demo")
        .arg("--store")
        .arg(&store)
        .env("SKRUN_GIT", &git)
        .env("SKRUN_TEST_GIT_LOG", root.join("git.log"))
        .env("GIT_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("GCM_INTERACTIVE", "always")
        .output()
        .expect("run the skrun binary");

    // The fake git of the test fails the command when a prompt program of
    // the environment is still there.
    assert_eq!(output.status.code(), Some(0));
    let log = fs::read_to_string(root.join("git.log")).unwrap();
    assert!(log.contains("GIT_ASKPASS=unset"), "{log}");
    assert!(log.contains("SSH_ASKPASS=unset"), "{log}");
    assert!(log.contains("SSH_ASKPASS_REQUIRE=unset"), "{log}");
    assert!(log.contains("GCM_INTERACTIVE=never"), "{log}");
    assert!(log.contains("GIT_TERMINAL_PROMPT=0"), "{log}");
    // The helper of the credentials never runs either.
    assert!(log.contains("credential.helper="), "{log}");
}

#[test]
fn the_configuration_of_git_is_never_read() {
    let root = temp_dir("github-isolation");
    let git = fake_git(&root);
    let store = root.join("store");
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    // The configuration of the user of git moves every address of GitHub
    // to another address.
    fs::write(
        home.join(".gitconfig"),
        "[url \"https://evil.example/\"]\n\tinsteadOf = https://github.com/\n",
    )
    .unwrap();

    // The fake git of the test fails the command when the configuration of
    // the user of git or the configuration of the environment of the shell
    // is in effect.
    let output = Command::new(BINARY)
        .arg("catalog")
        .arg("add")
        .arg("github:owner/repo@main//skills/demo")
        .arg("--store")
        .arg(&store)
        .env("SKRUN_GIT", &git)
        .env("SKRUN_TEST_GIT_LOG", root.join("git.log"))
        .env("HOME", &home)
        .env("GIT_CONFIG", home.join(".gitconfig"))
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "url.https://evil.example/.insteadOf")
        .env("GIT_CONFIG_VALUE_0", "https://github.com/")
        .env(
            "GIT_CONFIG_PARAMETERS",
            "'url.https://evil.example/.insteadOf=https://github.com/'",
        )
        .env("GIT_TEMPLATE_DIR", root.join("templates"))
        .env("GIT_DIR", root.join("other.git"))
        .output()
        .expect("run the skrun binary");

    assert_eq!(output.status.code(), Some(0));
    let log = fs::read_to_string(root.join("git.log")).unwrap();
    assert!(log.contains("GIT_CONFIG=unset"), "{log}");
    assert!(log.contains("GIT_CONFIG_GLOBAL=/dev/null"), "{log}");
    assert!(log.contains("GIT_CONFIG_NOSYSTEM=1"), "{log}");
    assert!(log.contains("GIT_CONFIG_COUNT=unset"), "{log}");
    assert!(log.contains("GIT_CONFIG_PARAMETERS=unset"), "{log}");
    assert!(log.contains("GIT_TEMPLATE_DIR=unset"), "{log}");
    assert!(log.contains("GIT_DIR=unset"), "{log}");
    // The rule of the configuration of the user of git is not in effect.
    assert!(!log.contains("insteadof"), "{log}");
}

/// The git command line of a test.
///
/// The command line holds a repository of its own, so the test never reaches
/// the network. Every command is written down, so the test can look at what
/// the fetch did.
struct FakeGit {
    /// The command lines that were run.
    calls: Mutex<Vec<Vec<String>>>,
    /// The folder that the command line worked in.
    directory: Mutex<Option<PathBuf>>,
    /// The commit that the command line reports.
    sha: String,
    /// The files that the fetch writes into the repository.
    files: Vec<(String, String)>,
    /// The symlinks that the fetch writes into the repository.
    links: Vec<(String, String)>,
    /// The command that fails.
    failing: Option<String>,
}

impl FakeGit {
    /// A git command line that holds a skill of its own.
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            directory: Mutex::new(None),
            sha: SHA.to_string(),
            files: vec![
                ("SKILL.md".to_string(), SKILL.to_string()),
                ("run.py".to_string(), SCRIPT.to_string()),
                ("skills/demo/SKILL.md".to_string(), SKILL.to_string()),
                ("skills/demo/run.py".to_string(), SCRIPT.to_string()),
            ],
            links: Vec::new(),
            failing: None,
        }
    }

    /// A git command line that reports another commit.
    fn with_sha(mut self, sha: &str) -> Self {
        self.sha = sha.to_string();
        self
    }

    /// A git command line whose repository holds a symlink.
    ///
    /// The files of the folder of the symlink are left out, so the symlink
    /// can take the place of the folder of the skill.
    fn with_link(mut self, name: &str, target: &Path) -> Self {
        let prefix = format!("{name}/");
        self.files.retain(|(file, _)| !file.starts_with(&prefix));
        self.links
            .push((name.to_string(), target.to_string_lossy().to_string()));
        self
    }

    /// A git command line whose repository holds no skill.
    ///
    /// The folder of the skill is there, but it holds no `SKILL.md`, so the
    /// add of the skill is refused.
    fn without_skill(mut self) -> Self {
        self.files = vec![("skills/demo/other.txt".to_string(), "other\n".to_string())];
        self
    }

    /// A git command line whose command fails.
    fn failing(mut self, command: &str) -> Self {
        self.failing = Some(command.to_string());
        self
    }

    /// The command lines that were run.
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }

    /// The folder that the command line worked in.
    fn directory(&self) -> PathBuf {
        self.directory.lock().unwrap().clone().expect("a folder")
    }
}

impl GitRunner for FakeGit {
    fn run(&self, args: &[&str], directory: &Path) -> anyhow::Result<GitOutput> {
        self.calls
            .lock()
            .unwrap()
            .push(args.iter().map(|arg| arg.to_string()).collect());
        let mut folder = self.directory.lock().unwrap();
        if folder.is_none() {
            *folder = Some(directory.to_path_buf());
        }
        drop(folder);

        let command = git_command(args);
        if self.failing.as_deref() == Some(command) {
            return Ok(GitOutput {
                success: false,
                stdout: String::new(),
                stderr: "the fetch failed\n".to_string(),
            });
        }
        match command {
            "fetch" => {
                for (name, text) in &self.files {
                    let path = directory.join(name);
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(path, text).unwrap();
                }
                for (name, target) in &self.links {
                    let path = directory.join(name);
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).unwrap();
                    }
                    std::os::unix::fs::symlink(target, &path).unwrap();
                }
                Ok(GitOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            }
            "rev-parse" => Ok(GitOutput {
                success: true,
                stdout: format!("{}\n", self.sha),
                stderr: String::new(),
            }),
            _ => Ok(GitOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            }),
        }
    }
}

/// The options of git that hold back every feature that runs code.
///
/// The options are the ones of the command of the store, so a test can
/// write the command line of a fetch without writing every option down.
const SAFE_OPTIONS: [&str; 14] = [
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "credential.helper=",
    "-c",
    "submodule.recurse=false",
    "-c",
    "fetch.recurseSubmodules=false",
    "-c",
    "filter.lfs.smudge=",
    "-c",
    "filter.lfs.process=",
    "-c",
    "filter.lfs.required=false",
];

/// The command line of one git command of the store.
fn safe(command: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = SAFE_OPTIONS
        .iter()
        .map(|option| option.to_string())
        .collect();
    for argument in command {
        args.push(argument.to_string());
    }
    args
}

/// The name of the git command of a command line.
///
/// The name of the command is the first argument that is not an option of
/// git itself.
fn git_command<'a>(args: &'a [&'a str]) -> &'a str {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "-c" {
            index += 2;
            continue;
        }
        return args[index];
    }
    ""
}

/// Add a skill of a repository of GitHub to a store.
fn add(git: &FakeGit, store: &Path, source: &str) -> store::AddResult {
    add_with(git, store, source, None, None).expect("add the skill to the store")
}

/// Add a skill of a repository of GitHub to a store, with a provenance of
/// the user.
fn add_with_from(
    git: &FakeGit,
    store: &Path,
    source: &str,
    from: Option<&str>,
) -> anyhow::Result<store::AddResult> {
    add_with(git, store, source, None, from)
}

/// Add a skill of a repository of GitHub to a store, with an id of the user.
fn add_with_id(git: &FakeGit, store: &Path, source: &str, id: Option<&str>) -> store::AddResult {
    add_with(git, store, source, id, None).expect("add the skill to the store")
}

/// Add a skill of a repository of GitHub to a store.
fn add_with(
    git: &FakeGit,
    store: &Path,
    source: &str,
    id: Option<&str>,
    from: Option<&str>,
) -> anyhow::Result<store::AddResult> {
    let source = store::parse_github_source(source).expect("read the source of GitHub");
    store::add_github_skill(
        &GitHubAddOptions {
            source,
            store: Some(store.to_path_buf()),
            id: id.map(|id| id.to_string()),
            from: from.map(|from| from.to_string()),
        },
        git,
    )
}

/// A fake git that holds a repository of its own.
///
/// The fake git writes the command line of every command into the log of the
/// test, so the test can look at what the fetch did. It never reaches the
/// network.
fn fake_git(root: &Path) -> PathBuf {
    let script = root.join("git");
    fs::write(
        &script,
        r#"#!/bin/sh
# The fake git of a test: it holds a repository of its own.
if [ "${GIT_TERMINAL_PROMPT:-}" != "0" ]; then
  echo "the prompt of git is not off" >&2
  exit 1
fi
if [ "${GIT_CONFIG_NOSYSTEM:-}" != "1" ]; then
  echo "the system configuration of git is not off" >&2
  exit 1
fi
if [ "${GIT_CONFIG_GLOBAL:-}" != "/dev/null" ]; then
  echo "the configuration of the user of git is not off" >&2
  exit 1
fi
if [ -n "${GIT_ASKPASS:-}" ]; then
  echo "the prompt program of git is not off" >&2
  exit 1
fi
if [ -n "${SSH_ASKPASS:-}" ]; then
  echo "the prompt program of ssh is not off" >&2
  exit 1
fi
if [ -n "${SSH_ASKPASS_REQUIRE:-}" ]; then
  echo "the prompt program of ssh is not off" >&2
  exit 1
fi
if [ "${GCM_INTERACTIVE:-}" != "never" ]; then
  echo "the credential manager of git is not off" >&2
  exit 1
fi
if [ -n "${GIT_CONFIG:-}" ]; then
  echo "the configuration of the environment is not off" >&2
  exit 1
fi
if [ -n "${GIT_CONFIG_COUNT:-}" ]; then
  echo "the configuration of the environment is not off" >&2
  exit 1
fi
if [ -n "${GIT_CONFIG_PARAMETERS:-}" ]; then
  echo "the configuration of the environment is not off" >&2
  exit 1
fi
if [ -n "${GIT_TEMPLATE_DIR:-}" ]; then
  echo "the folder of the templates of git is not off" >&2
  exit 1
fi
if [ -n "${GIT_DIR:-}" ]; then
  echo "the repository of the environment is not off" >&2
  exit 1
fi
command=""
previous=""
for argument in "$@"; do
  if [ "$previous" = "-c" ]; then
    previous="$argument"
    continue
  fi
  case "$argument" in
    -c)
      previous="-c"
      continue
      ;;
  esac
  command="$argument"
  break
done
case "$command" in
  fetch|checkout)
    case " $* " in
      *" core.hooksPath=/dev/null "*) ;;
      *)
        echo "the hooks of the repository are not off" >&2
        exit 1
        ;;
    esac
    case " $* " in
      *" credential.helper= "*) ;;
      *)
        echo "the helper of the credentials is not off" >&2
        exit 1
        ;;
    esac
    ;;
esac
if [ -n "${SKRUN_TEST_GIT_LOG:-}" ]; then
  printf '%s\n' "$*" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GIT_CONFIG=%s\n' "${GIT_CONFIG:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GIT_CONFIG_GLOBAL=%s\n' "${GIT_CONFIG_GLOBAL:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GIT_CONFIG_NOSYSTEM=%s\n' "${GIT_CONFIG_NOSYSTEM:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GIT_CONFIG_COUNT=%s\n' "${GIT_CONFIG_COUNT:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GIT_CONFIG_PARAMETERS=%s\n' "${GIT_CONFIG_PARAMETERS:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GIT_TEMPLATE_DIR=%s\n' "${GIT_TEMPLATE_DIR:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GIT_DIR=%s\n' "${GIT_DIR:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GIT_ASKPASS=%s\n' "${GIT_ASKPASS:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'SSH_ASKPASS=%s\n' "${SSH_ASKPASS:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'SSH_ASKPASS_REQUIRE=%s\n' "${SSH_ASKPASS_REQUIRE:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GCM_INTERACTIVE=%s\n' "${GCM_INTERACTIVE:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  printf 'GIT_TERMINAL_PROMPT=%s\n' "${GIT_TERMINAL_PROMPT:-unset}" >> "$SKRUN_TEST_GIT_LOG"
  if command -v git >/dev/null 2>&1; then
    git config --list 2>/dev/null | sed 's/^/CONFIG /' >> "$SKRUN_TEST_GIT_LOG"
  fi
fi
case "$command" in
  init)
    mkdir -p .git
    ;;
  fetch)
    mkdir -p skills/demo
    printf '%s\n' '---' 'name: demo' 'description: A demo skill.' '---' '' '# Demo' > SKILL.md
    printf '%s\n' 'import os' '' 'KEY = "AKIAIOSFODNN7EXAMPLE"' 'print(KEY)' > run.py
    mkdir -p skills/demo
    cp SKILL.md skills/demo/SKILL.md
    cp run.py skills/demo/run.py
    ;;
  rev-parse)
    printf '%s\n' '0123456789abcdef0123456789abcdef01234567'
    ;;
esac
exit 0
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// Whether the store holds no skill at all.
///
/// The folder of the skills of the store may be there, but it holds no
/// folder of a skill.
fn store_skills_are_empty(store: &Path) -> bool {
    let skills = store.join("skills");
    if !skills.exists() {
        return true;
    }
    let Ok(entries) = skills.read_dir() else {
        return false;
    };
    entries.count() == 0
}

/// A fresh directory below the system temporary directory.
fn temp_dir(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "skrun-github-test-{name}-{}-{now}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
