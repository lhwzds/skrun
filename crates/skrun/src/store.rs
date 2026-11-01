//! # skrun
//!
//! Store owns the local store of skills that come from outside.
//!
//! A skill from another repository or from a registry is never run. It is
//! copied into the store, hashed and scanned, so the user can look at what
//! arrived before anything of it is used.
//!
//! ## Owns
//! - staging of a local skill directory or of a zip archive
//! - the fetch of a repository of GitHub into a temporary directory
//! - refusal of an unsafe zip archive
//! - refusal of a source that is a symlink
//! - refusal of a name that is not text
//! - the id of an added skill, taken from the real path of its source
//! - the content hash of a staged skill
//! - the name of the folder of every version of a skill
//! - the read-only copy of a stored skill
//! - the newest version of every stored id and the text of it
//! - the folder of a stored version and the files that it holds
//! - the text of a file of a stored version
//! - the threat record of every added version
//! - the risk rules of the scan and the tier they suggest
//! - the finding of a file that was not read and of a file that cannot be
//!   parsed
//! - the finding of a file that hides a module of the standard library
//! - the lock of a skill id while it is added
//! - the modes of the folders and of the files of the store
//!
//! ## Must Not
//! - execute anything that comes from an added skill
//! - reach the network for anything but the fetch of a repository of GitHub
//! - overwrite a stored version
//! - print a value that looks like a secret
//! - skip a file of the scan because it is not text
//! - read a file that is not inside the folder of a version
//! - follow a symlink of a stored version
//! - follow a symlink of a folder of the store
//! - follow a symlink of the subpath of a repository
//! - read the configuration of git of the machine of the user
//!
//! ## Inputs
//! - a local skill directory or a zip archive
//! - a repository of GitHub, with its owner, its name, its ref and its subpath
//! - a store root directory
//! - the id of a stored skill and the name of a file of a version
//!
//! ## Outputs
//! - a read-only stored skill copy
//! - a threat record
//! - a store summary
//! - the matches of a search of the store
//! - the folder of a stored version
//! - the text of a file of a stored version
//!
//! ## Depends On
//! - libc
//! - regex
//! - serde_json
//! - sha2
//! - zip
//!
//! ## Verify
//! - cargo test -p skrun

use crate::catalog;
use crate::risk;
use crate::runtime::validate_id;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Schema version written into every threat record.
pub const RECORD_SCHEMA_VERSION: u32 = 1;

/// The prefix of a source of the `catalog add` command that names a
/// repository of GitHub.
pub const GITHUB_PREFIX: &str = "github:";

/// Environment variable that holds the name of the git program that fetches
/// a repository of GitHub.
///
/// The variable is the way to a git that is not the one of the path, so a
/// test can fetch a repository without the network.
pub const GIT_ENV: &str = "SKRUN_GIT";

/// Environment variable that holds the store root.
pub const STORE_ENV: &str = "SKRUN_STORE";

/// The variables of the environment that are removed before a git command.
///
/// The variables name a configuration file of git, a folder of templates or
/// a repository of its own, so the configuration of the shell of the user
/// cannot move the fetch of a repository away from the address of GitHub.
const GIT_ISOLATED_VARIABLES: [&str; 14] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_SYSTEM",
    "GIT_DIR",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_INDEX_FILE",
    "GIT_NAMESPACE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_TEMPLATE_DIR",
    "GIT_WORK_TREE",
];

/// The options of git that hold back every feature that runs code.
///
/// The hooks of the repository never run, the helpers of the credentials
/// never run, the submodules are never fetched and the filters of the
/// files never run, because a repository of somebody else must not run
/// anything on the machine of the user.
const GIT_SAFE_OPTIONS: [&str; 14] = [
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

/// Name of the skill entrypoint inside a skill root.
pub const SKILL_FILE: &str = "SKILL.md";

/// Largest uncompressed size of a zip archive that is accepted.
pub const MAX_ARCHIVE_BYTES: u64 = 50 * 1000 * 1000;

/// Largest number of entries a zip archive may hold.
pub const MAX_ARCHIVE_ENTRIES: usize = 5_000;

/// Largest compression ratio of a single zip entry.
pub const MAX_COMPRESSION_RATIO: u64 = 100;

/// Number of hex characters of the content hash that name a version.
pub const HASH_LENGTH: usize = 16;

/// The mode of the folders of the store that hold its state.
///
/// The store holds what the user brought in, so only the user of the store
/// can read the folders of the store.
const PRIVATE_DIR_MODE: u32 = 0o700;

/// The mode of the files of a stored skill.
///
/// A stored skill never changes, so the files of a stored skill are read
/// only for the user of the store.
const STORED_FILE_MODE: u32 = 0o400;

/// The mode of the folders of a stored skill.
///
/// A stored skill never changes, so the folders of a stored skill are read
/// only for the user of the store.
const STORED_DIR_MODE: u32 = 0o500;

/// The mode of the records of the store.
///
/// A record holds what the user brought in, so only the user of the store
/// can read a record.
const RECORD_MODE: u32 = 0o600;

/// How long an add waits for the lock of a skill id.
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// How long an add waits between two tries for the lock of a skill id.
const LOCK_RETRY: Duration = Duration::from_millis(50);

/// The time after which a lock belongs to an add that never finished.
///
/// A lock that is older than ten minutes is stale, so the lock is removed
/// and the add goes on.
const LOCK_STALE_AFTER: Duration = Duration::from_secs(600);

/// Folders that are never copied out of a skill directory.
const SKIPPED_FOLDERS: [&str; 5] = [".git", ".venv", "__pycache__", "node_modules", "target"];

/// Extensions that mark a file as a binary payload.
const BINARY_EXTENSIONS: [&str; 10] = [
    "exe", "dll", "so", "dylib", "dmg", "pkg", "msi", "jar", "node", "wasm",
];

/// Magic numbers of the binary formats a skill may hold.
const BINARY_MAGICS: [&[u8]; 7] = [
    b"\x7fELF",
    &[0xfe, 0xed, 0xfa, 0xce],
    &[0xfe, 0xed, 0xfa, 0xcf],
    &[0xce, 0xfa, 0xed, 0xfe],
    &[0xcf, 0xfa, 0xed, 0xfe],
    &[0xca, 0xfe, 0xba, 0xbe],
    b"MZ",
];

/// How serious a finding of the threat record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// A rule that found something that must not run.
    High,
    /// A rule that found something that needs a look.
    Medium,
    /// A hint that needs a look.
    Low,
}

/// The name of a severity, as it is printed.
fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::High => "high",
        Severity::Medium => "medium",
        Severity::Low => "low",
    }
}

/// The sandbox tier that the risk scan suggests for a skill.
///
/// The tier says how much room a skill needs. A skill with a high finding
/// never runs outside a microVM, a skill with a script that reaches outside
/// of the process runs in a container, a skill that touches the file system
/// runs in a sandbox and a skill that only computes runs in a plain process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    /// A high finding: never run outside a microVM.
    L3,
    /// A script that is not python or that reaches outside of the process.
    L2,
    /// A script that touches the file system.
    L1,
    /// A script that only computes.
    L0,
    /// No script at all: the skill is guidance and nothing to run.
    #[default]
    #[serde(rename = "none")]
    None,
}

impl Tier {
    /// The name of the tier, as it is printed and as it is stored.
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::L3 => "L3",
            Tier::L2 => "L2",
            Tier::L1 => "L1",
            Tier::L0 => "L0",
            Tier::None => "none",
        }
    }

    /// The name of a tier, when the name is one of the tiers.
    ///
    /// The name of a tier is the name it is printed with, so a tier of a
    /// record and a tier of the command line are the same tier.
    pub fn from_name(name: &str) -> Option<Tier> {
        match name {
            "L0" => Some(Tier::L0),
            "L1" => Some(Tier::L1),
            "L2" => Some(Tier::L2),
            "L3" => Some(Tier::L3),
            "none" => Some(Tier::None),
            _ => None,
        }
    }

    /// The rank of the tier, where a higher rank needs more room.
    ///
    /// A skill without a tier is guidance and needs no room at all, so its
    /// rank is the lowest one.
    pub fn rank(&self) -> u8 {
        match self {
            Tier::None => 0,
            Tier::L0 => 1,
            Tier::L1 => 2,
            Tier::L2 => 3,
            Tier::L3 => 4,
        }
    }

    /// Whether the tier is at most `limit`.
    ///
    /// A version of the tier `none` is at most every tier, so a skill that
    /// is only guidance always passes a filter of the tier.
    pub fn at_most(&self, limit: Tier) -> bool {
        self.rank() <= limit.rank()
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<Tier> for String {
    fn from(tier: Tier) -> String {
        tier.as_str().to_string()
    }
}

impl PartialEq<Tier> for String {
    fn eq(&self, other: &Tier) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<String> for Tier {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<&str> for Tier {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// One finding of the threat record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The rule that found the issue.
    pub rule: String,
    /// How serious the issue is.
    pub severity: Severity,
    /// The path of the file, relative to the skill root.
    pub path: String,
    /// The line of the file, when the rule points at one.
    pub line: Option<u64>,
    /// What the rule found, when the rule can say more than the path.
    ///
    /// A finding that hides a module of the standard library keeps the name
    /// of the module that it hides, so the user knows what the file of the
    /// skill stands in the way of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One script of the threat record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredScript {
    /// The path of the script, relative to the skill root.
    pub path: String,
    /// The language of the script.
    pub language: String,
    /// The modules the script imports.
    pub imports: Vec<String>,
}

/// One stored version of a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredVersion {
    /// The content hash that names the version.
    ///
    /// The name is the short hash of the content of the version. When a
    /// folder of that name already holds another version, a longer part of
    /// the hash names the version, so two versions never share a folder.
    pub hash: String,
    /// The whole content hash of the version.
    pub full_hash: String,
    /// Where the skill came from, when the user said so.
    pub from: Option<String>,
    /// When the version was added, as RFC 3339 UTC.
    pub added_at: String,
    /// The number of stored files.
    pub files: u64,
    /// The number of stored bytes.
    pub bytes: u64,
    /// The name of the skill, from its frontmatter.
    pub name: Option<String>,
    /// The description of the skill, from its frontmatter.
    pub description: Option<String>,
    /// The scripts of the skill.
    pub scripts: Vec<StoredScript>,
    /// The findings of the threat scan.
    pub findings: Vec<Finding>,
    /// The tier that the risk scan suggests for the version.
    #[serde(default)]
    pub tier: Tier,
    /// The reasons of the tier.
    #[serde(default)]
    pub tier_reasons: Vec<String>,
}

/// The threat record of one skill id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreatRecord {
    /// The schema of the record.
    pub schema_version: u32,
    /// The id of the skill.
    pub id: String,
    /// Every added version, the newest one last.
    pub versions: Vec<StoredVersion>,
}

/// One stored skill, as it is listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreSummary {
    /// The id of the skill.
    pub id: String,
    /// The hash of the newest version.
    pub hash: String,
    /// The tier that the newest version suggests.
    pub tier: Tier,
    /// The number of high findings of the newest version.
    pub high: u64,
    /// The number of medium findings of the newest version.
    pub medium: u64,
    /// The number of low findings of the newest version.
    pub low: u64,
}

/// What the `catalog add` command needs.
#[derive(Debug, Clone)]
pub struct AddOptions {
    /// The skill directory or the zip archive to add.
    pub source: PathBuf,
    /// The store root, when the user named one.
    pub store: Option<PathBuf>,
    /// The id of the skill, when the user named one.
    pub id: Option<String>,
    /// Where the skill came from.
    pub from: Option<String>,
}

/// What the `catalog add` command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddResult {
    /// The id of the skill.
    pub id: String,
    /// Whether the very same content was already stored.
    pub already_stored: bool,
    /// Whether the record of the stored version was refreshed.
    pub refreshed: bool,
    /// The version of the skill.
    pub version: StoredVersion,
}

/// The store root of the store.
///
/// The root is the directory the user named, else the directory of the
/// `SKRUN_STORE` environment variable, else the store below the home
/// directory of the user.
pub fn resolve_store_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(root) = explicit {
        return Ok(root.to_path_buf());
    }
    if let Some(root) = std::env::var_os(STORE_ENV)
        && !root.is_empty()
    {
        return Ok(PathBuf::from(root));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".skrun").join("store"))
}

/// The path of the threat record of one skill id.
pub fn record_path(root: &Path, id: &str) -> PathBuf {
    root.join("records").join(format!("{id}.json"))
}

/// Add a skill to the store.
///
/// The skill is staged, hashed and scanned first. Only a skill that passes
/// every check is copied into the store, and the copy is read-only, so a
/// stored skill never changes behind the back of the user.
///
/// The source itself is never a symlink, because a symlink can point at
/// anything and the store only takes what the user can see.
///
/// The id of the skill is the one the user named, else the name of the
/// source, so two skills of two authors that share a frontmatter name stay
/// two skills. The name of the source comes from the real path of the
/// source, so `catalog add .` names the skill after the folder the user is
/// in. The name of the frontmatter is kept in the record, because it is the
/// name the skill shows under.
///
/// A skill with the very same content is already stored, so it is not copied
/// a second time and no second version is written. The record of a stored
/// version is refreshed with the findings, the tier and the reasons of the
/// tier of a fresh scan, so a record that was written by an older rule holds
/// the findings of today. The provenance of the skill is the one of the add
/// of today, so a skill that is stored again names where it came from now.
/// The hash and the time of the version stay the same, because the content
/// did not change. A stored copy whose version is missing from the record
/// repairs the record instead.
///
/// A source that names a repository of GitHub is fetched into a temporary
/// directory first, and the skill of the repository is added from there, so
/// the skill of a repository goes through the same path as a skill of the
/// machine of the user.
pub fn add_skill(options: &AddOptions) -> Result<AddResult> {
    add_skill_with(options, &SystemGit)
}

/// Add a skill to the store with a git command line of its own.
///
/// The command line is the one that fetches a repository of GitHub, so a
/// test can hold a repository of its own and never reaches the network.
pub fn add_skill_with(options: &AddOptions, runner: &dyn GitRunner) -> Result<AddResult> {
    if let Some(source) = options.source.to_str()
        && source.starts_with(GITHUB_PREFIX)
    {
        let source = parse_github_source(source)?;
        return add_github_skill(
            &GitHubAddOptions {
                source,
                store: options.store.clone(),
                id: options.id.clone(),
                from: options.from.clone(),
            },
            runner,
        );
    }
    add_local_skill(options)
}

/// Add a skill of the machine of the user to the store.
///
/// The skill is staged, hashed and scanned first. Only a skill that passes
/// every check is copied into the store, and the copy is read-only, so a
/// stored skill never changes behind the back of the user.
fn add_local_skill(options: &AddOptions) -> Result<AddResult> {
    let root = resolve_store_root(options.store.as_deref())?;
    let source = options.source.as_path();
    let Ok(metadata) = fs::symlink_metadata(source) else {
        bail!("source `{}` does not exist", catalog::redact_path(source));
    };
    if metadata.file_type().is_symlink() {
        bail!(
            "source `{}` is a symlink, a symlink is never followed",
            catalog::redact_path(source)
        );
    }

    // The identity of the source comes from the real path of the source, so
    // `catalog add .` names the skill after the folder the user is in and
    // not after the `.` the user typed.
    let real = fs::canonicalize(source)
        .with_context(|| format!("resolve source `{}`", catalog::redact_path(source)))?;

    let staging = Staging::new()?;
    let links = stage_source(source, &staging.path)?;
    let skill_root = skill_root(&staging.path)?;
    let entrypoint = skill_root.join(SKILL_FILE);
    let frontmatter =
        catalog::parse_frontmatter(&fs::read_to_string(&entrypoint).unwrap_or_default());
    let id = resolve_id(
        options.id.as_deref(),
        frontmatter.name.as_deref(),
        source,
        &real,
    )?;
    if let Err(error) = validate_id(&id) {
        bail!(
            "skill id `{}` is not a valid id: {error}",
            catalog::redact(&id)
        );
    }

    // The lock of the skill id is held until the add is done, also when the
    // add fails, so two adds of the same skill id never write the same
    // record at the same time.
    let _lock = Lock::acquire(&root, &id)?;

    // The store holds what the user brought in, so the folders of the store
    // are private to the user of the store.
    create_private_dir(&root)?;
    let skills_root = root.join("skills");
    create_private_dir(&skills_root)?;
    let id_folder = skills_root.join(&id);
    let id_folder_is_new = !id_folder.exists();
    create_private_dir(&id_folder)?;

    // The add is a transaction: the folder of the skill id and the folder
    // of the version go away again when the add fails, so the store holds
    // nothing of an add that failed.
    let mut transaction = StoreTransaction::new();
    if id_folder_is_new {
        transaction.remember_id_folder(&id_folder);
    }

    let outcome = (|| -> Result<AddResult> {
        let (_short_hash, full_hash, files, bytes) = content_hash(&skill_root)?;
        let scan = scan_skill(&skill_root)?;
        let Scan {
            scripts,
            findings: scanned,
            opens_file,
        } = scan;
        let mut findings = scanned;
        let prefix = skill_prefix(&skill_root, &staging.path);
        for link in links {
            let Some(path) = strip_prefix(&link, &prefix) else {
                continue;
            };
            findings.push(Finding {
                rule: "symlink".to_string(),
                severity: Severity::Medium,
                path: catalog::redact(&path),
                line: None,
                detail: None,
            });
        }
        findings.sort_by(|left, right| {
            left.severity
                .cmp(&right.severity)
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| left.line.cmp(&right.line))
        });
        let (tier, tier_reasons) = suggest_tier(&scripts, opens_file, &findings);

        // The folder of the version is named after the hash of the version. A
        // folder of that name that holds another version gets a longer name, so
        // two versions of a skill never share a folder.
        let name = version_name(&root, &id, &full_hash)?;
        let version = StoredVersion {
            hash: name.clone(),
            full_hash,
            from: options.from.as_deref().map(catalog::redact),
            added_at: rfc3339(SystemTime::now()),
            files,
            bytes,
            name: frontmatter.name.as_deref().map(catalog::redact),
            description: frontmatter.description.as_deref().map(catalog::redact),
            scripts: scripts
                .iter()
                .map(|scanned| scanned.script.clone())
                .collect(),
            findings,
            tier,
            tier_reasons,
        };

        let target = skills_root.join(&id).join(&name);
        if !target.exists() {
            store_copy(&skill_root, &skills_root.join(&id), &target)?;
            // The folder of the version belongs to this add, so it goes away
            // again when the add fails.
            transaction.remember_version(&target);
        }

        let mut record = load_record(&root, &id)?;
        // The version of the record is the one of the folder of the version, so
        // a folder that holds another version never takes the place of a
        // version that is already stored.
        let known = record
            .versions
            .iter()
            .position(|stored| stored.hash == version.hash);
        if let Some(index) = known {
            // The content is already stored, so the record of the version is
            // refreshed with the scripts, the findings, the tier and the reasons
            // of the tier of the scan of today. A record that was written by an
            // older rule holds the scripts and the findings of today after the
            // refresh. The hash and the time of the version stay the same,
            // because the content did not change.
            let stored = &mut record.versions[index];
            let refreshed = stored.scripts != version.scripts
                || stored.findings != version.findings
                || stored.tier != version.tier
                || stored.tier_reasons != version.tier_reasons;
            // The provenance of the skill is the one of the add of today, so a
            // skill that is stored again names where it came from now, even
            // when the content is the very same content. A skill that is added
            // without a provenance keeps the one that it has.
            let provenance = version.from.clone();
            let provenance_changed = provenance.is_some() && stored.from != provenance;
            if refreshed || provenance_changed {
                stored.scripts = version.scripts.clone();
                stored.findings = version.findings.clone();
                stored.tier = version.tier;
                stored.tier_reasons = version.tier_reasons.clone();
                if provenance_changed {
                    stored.from = provenance;
                }
                write_record(&root, &id, &record)?;
            }
            let stored = record.versions[index].clone();
            return Ok(AddResult {
                id,
                already_stored: true,
                refreshed: refreshed || provenance_changed,
                version: stored,
            });
        }

        // The record lost the version of the copy that is there: the record is
        // repaired instead of the version being stored again.
        record.versions.push(version.clone());
        write_record(&root, &id, &record)?;

        Ok(AddResult {
            id,
            already_stored: false,
            refreshed: false,
            version,
        })
    })();

    // A failed add leaves nothing of itself in the store.
    if outcome.is_err() {
        transaction.rollback();
    }
    outcome
}

/// The folders of the store that one add creates.
///
/// An add is a transaction: when the add fails, the folders it created go
/// away again, so the store holds nothing of an add that failed.
struct StoreTransaction {
    /// The folder of the skill id, when the add created it.
    id_folder: Option<PathBuf>,
    /// The folder of the version, when the add created it.
    version_folder: Option<PathBuf>,
}

impl StoreTransaction {
    /// A transaction that created nothing yet.
    fn new() -> Self {
        Self {
            id_folder: None,
            version_folder: None,
        }
    }

    /// Remember the folder of the skill id, when the add created it.
    fn remember_id_folder(&mut self, path: &Path) {
        self.id_folder = Some(path.to_path_buf());
    }

    /// Remember the folder of the version, when the add created it.
    fn remember_version(&mut self, path: &Path) {
        self.version_folder = Some(path.to_path_buf());
    }

    /// Take the folders of the add away again.
    ///
    /// The folder of the version goes away, and the folder of the skill id
    /// goes away too when it is empty, so a version that was stored before
    /// is never lost.
    fn rollback(self) {
        if let Some(folder) = &self.version_folder {
            let _ = remove_tree(folder);
        }
        if let Some(folder) = &self.id_folder {
            let _ = fs::remove_dir(folder);
        }
    }
}

/// A repository of GitHub that a skill comes from.
///
/// The source is written as `github:<owner>/<repo>@<ref>//<subpath>`. The
/// ref is optional and names the branch, the tag or the commit that is
/// fetched, and the subpath is optional and names the folder of the
/// repository that holds the skill. A source without a ref fetches the
/// default branch of the repository, and a source without a subpath takes
/// the whole repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubSource {
    /// The owner of the repository.
    pub owner: String,
    /// The name of the repository.
    pub repo: String,
    /// The ref that is fetched, when the user named one.
    pub reference: Option<String>,
    /// The folder of the repository that holds the skill.
    pub subpath: Option<String>,
}

/// What the `catalog add` command needs to add a skill of GitHub.
#[derive(Debug, Clone)]
pub struct GitHubAddOptions {
    /// The repository of GitHub that holds the skill.
    pub source: GitHubSource,
    /// The store root, when the user named one.
    pub store: Option<PathBuf>,
    /// The id of the skill, when the user named one.
    pub id: Option<String>,
    /// Where the skill came from, when the user said so.
    ///
    /// The provenance of a source of GitHub is the repository with the
    /// commit that was fetched, so this is always empty for a source of
    /// GitHub.
    pub from: Option<String>,
}

/// The result of one git command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    /// Whether the command ended well.
    pub success: bool,
    /// The text that the command wrote to its standard output.
    pub stdout: String,
    /// The text that the command wrote to its standard error.
    pub stderr: String,
}

/// The git command line that fetches a repository.
///
/// The trait is the way to a repository without the network, so a test can
/// hold a repository of its own and never reaches GitHub.
pub trait GitRunner {
    /// Run one git command in a directory and return what it wrote.
    fn run(&self, args: &[&str], directory: &Path) -> Result<GitOutput>;
}

/// The git command line of the machine of the user.
///
/// The command line never asks for a password and never reads the system
/// configuration of git, so a fetch of a repository of somebody else cannot
/// ask the user for anything and cannot be moved by the configuration of the
/// machine.
///
/// The configuration of the user of git is never read either, and neither is
/// the configuration that the environment of the shell hands to git, so a
/// rule like `url.<base>.insteadOf` of the machine of the user cannot move
/// the fetch away from the address of GitHub.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemGit;

impl GitRunner for SystemGit {
    fn run(&self, args: &[&str], directory: &Path) -> Result<GitOutput> {
        let program = git_program();
        let mut command = std::process::Command::new(&program);
        command.args(args).current_dir(directory);
        // The command line never asks for a password: the terminal of the
        // user never prompts and the prompt programs of the environment
        // are gone, so a fetch of a repository of somebody else cannot ask
        // the user for anything.
        command.env("GIT_TERMINAL_PROMPT", "0");
        command.env_remove("GIT_ASKPASS");
        command.env_remove("SSH_ASKPASS");
        command.env_remove("SSH_ASKPASS_REQUIRE");
        // The credential manager of git never asks anything either.
        command.env("GCM_INTERACTIVE", "never");
        // The configuration of the system of git is never read.
        command.env("GIT_CONFIG_NOSYSTEM", "1");
        // The configuration of the user of git is never read, so a rule of
        // the configuration of the machine cannot move the fetch away from
        // the address of GitHub.
        command.env("GIT_CONFIG_GLOBAL", "/dev/null");
        // The configuration of the environment of the shell is never read
        // either, so a variable of the shell cannot move the fetch away
        // from the address of GitHub.
        for variable in GIT_ISOLATED_VARIABLES {
            command.env_remove(variable);
        }
        for (name, _) in std::env::vars_os() {
            let name = name.to_string_lossy();
            if name.starts_with("GIT_CONFIG_KEY_") || name.starts_with("GIT_CONFIG_VALUE_") {
                command.env_remove(name.as_ref());
            }
        }
        let output = command
            .output()
            .with_context(|| format!("run `{}`", catalog::redact(&program)))?;
        Ok(GitOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

/// The git program that fetches a repository.
///
/// The program is the one of the `SKRUN_GIT` variable, else the `git` of the
/// path of the machine.
pub fn git_program() -> String {
    std::env::var(GIT_ENV)
        .ok()
        .map(|program| program.trim().to_string())
        .filter(|program| !program.is_empty())
        .unwrap_or_else(|| "git".to_string())
}

/// Read a source of a repository of GitHub.
///
/// The source is `github:<owner>/<repo>[@<ref>][//<subpath>]`. The ref and
/// the subpath are optional, and every part of the source is checked,
/// because the owner and the repository end up in the address of the fetch
/// and the ref ends up in the command line of the fetch.
pub fn parse_github_source(source: &str) -> Result<GitHubSource> {
    let Some(rest) = source.strip_prefix(GITHUB_PREFIX) else {
        bail!("`{}` is not a source of GitHub", catalog::redact(source));
    };
    // The subpath is behind a double slash, so the repository and its ref
    // are read first.
    let (repository, subpath) = match rest.find("//") {
        Some(position) => (&rest[..position], Some(rest[position + 2..].to_string())),
        None => (rest, None),
    };
    // The ref is behind an at sign, so the owner and the repository are read
    // first.
    let (repository, reference) = match repository.find('@') {
        Some(position) => (
            &repository[..position],
            Some(repository[position + 1..].to_string()),
        ),
        None => (repository, None),
    };
    let Some((owner, repo)) = repository.split_once('/') else {
        bail!("a source of GitHub is `github:<owner>/<repo>[@<ref>][//<subpath>]`");
    };
    check_github_name(owner, "owner")?;
    check_github_repo(repo)?;
    let reference = match reference {
        Some(reference) => {
            check_github_ref(&reference)?;
            Some(reference)
        }
        None => None,
    };
    let subpath = match subpath {
        Some(subpath) => {
            check_github_subpath(&subpath)?;
            Some(subpath)
        }
        None => None,
    };
    Ok(GitHubSource {
        owner: owner.to_string(),
        repo: repo.to_string(),
        reference,
        subpath,
    })
}

/// Check the owner or the name of a repository.
///
/// The name starts with a letter or a number and holds only letters,
/// numbers, a dot, a dash and an underscore after that, so it can never
/// be an option of the command line and can never change the address of
/// the fetch.
fn check_github_name(name: &str, field: &str) -> Result<()> {
    if name.is_empty() {
        bail!("the {field} of a source of GitHub cannot be empty");
    }
    let mut characters = name.chars();
    let first = characters.next().unwrap_or_default();
    if !first.is_ascii_alphanumeric() {
        bail!(
            "the {field} `{}` of a source of GitHub must start with a letter or a number",
            catalog::redact(name)
        );
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-'))
    {
        bail!(
            "the {field} `{}` of a source of GitHub holds a character that is not allowed",
            catalog::redact(name)
        );
    }
    Ok(())
}

/// Check the name of a repository of GitHub.
///
/// The name of the repository ends up in the address of the fetch, and the
/// fetch adds `.git` to that address, so a name that ends with `.git`
/// would end up with `.git` twice.
fn check_github_repo(repo: &str) -> Result<()> {
    check_github_name(repo, "repository")?;
    if repo.ends_with(".git") {
        bail!(
            "the repository `{}` of a source of GitHub cannot end with `.git`",
            catalog::redact(repo)
        );
    }
    Ok(())
}

/// Check the ref of a repository of GitHub.
///
/// The ref holds only letters, numbers, a dot, a dash, an underscore and a slash, it never
/// starts with a dash and it never holds two dots in a row, so it can never
/// leave the repository and can never be an option of the command line of
/// the fetch.
fn check_github_ref(reference: &str) -> Result<()> {
    if reference.is_empty() {
        bail!("the ref of a source of GitHub cannot be empty");
    }
    if reference.starts_with('-') {
        bail!("the ref of a source of GitHub cannot start with a dash");
    }
    if !reference.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '/' | '-' | '_')
    }) {
        bail!(
            "the ref `{}` of a source of GitHub holds a character that is not allowed",
            catalog::redact(reference)
        );
    }
    if reference.contains("..") {
        bail!("the ref of a source of GitHub cannot hold `..`");
    }
    Ok(())
}

/// Check the subpath of a repository of GitHub.
///
/// The subpath is a path inside the repository, so it never starts with a
/// slash and every part of it is a name of a folder: the part holds only
/// letters, numbers, a dot, a dash and an underscore, it never starts with
/// a dash and it is never the current folder or the folder above it.
fn check_github_subpath(subpath: &str) -> Result<()> {
    if subpath.is_empty() {
        bail!("the subpath of a source of GitHub cannot be empty");
    }
    if subpath.starts_with('/') {
        bail!("the subpath of a source of GitHub cannot start with a slash");
    }
    for part in subpath.split('/') {
        if part.is_empty() {
            bail!(
                "the subpath `{}` of a source of GitHub is not a path inside the repository",
                catalog::redact(subpath)
            );
        }
        if part == "." {
            bail!("the subpath of a source of GitHub cannot hold `.`");
        }
        if part == ".." {
            bail!("the subpath of a source of GitHub cannot hold `..`");
        }
        if part.starts_with('-') {
            bail!("the subpath of a source of GitHub cannot hold a part that starts with a dash");
        }
        if !part.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
        }) {
            bail!(
                "the subpath `{}` of a source of GitHub holds a character that is not allowed",
                catalog::redact(subpath)
            );
        }
    }
    Ok(())
}

/// Check that every part of the subpath of a repository is a real folder.
///
/// A repository can hold a symlink in the middle of the subpath of a skill,
/// and a symlink can point at anything, so the files of the skill would come
/// from outside the checkout. Every part of the subpath is a real folder of
/// the checkout before the folder of the skill is opened.
fn check_github_subpath_links(root: &Path, subpath: &str) -> Result<()> {
    let mut path = root.to_path_buf();
    let mut relative = PathBuf::new();
    for part in subpath.split('/') {
        path.push(part);
        relative.push(part);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            bail!(
                "the folder `{}` of the repository does not exist",
                catalog::redact_path(&relative)
            );
        };
        if metadata.file_type().is_symlink() {
            bail!(
                "the folder `{}` of the repository is a symlink, a symlink is never followed",
                catalog::redact_path(&relative)
            );
        }
    }
    Ok(())
}

/// Add a skill of a repository of GitHub to the store.
///
/// The repository is fetched into a temporary directory and the skill of the
/// subpath is added from there, so the skill of a repository goes through the
/// same checks, the same scan and the same tier as a skill of the machine of
/// the user. Nothing of the repository is ever run.
///
/// The provenance of the skill is the repository with the commit that was
/// fetched and the subpath of the skill, so the record of the store names
/// the exact content that was stored. The provenance of a source of GitHub
/// is never the one of the user, because the record must name the commit
/// that was fetched.
///
/// No part of the subpath of the skill is a symlink, so the files of the
/// skill come from the checkout of the repository and not from somewhere
/// else of the machine of the user.
///
/// The temporary directory of the fetch goes away with the command, also
/// when the add fails.
pub fn add_github_skill(options: &GitHubAddOptions, runner: &dyn GitRunner) -> Result<AddResult> {
    // The provenance of a source of GitHub is the repository with the
    // commit that was fetched, so the user cannot name another one.
    if options.from.is_some() {
        bail!("a source of GitHub has its own provenance, `--from` is not allowed");
    }
    // The id of the skill is known before the repository is fetched, so a
    // name that is no id never reaches the network.
    let id = match options.id.clone() {
        Some(id) => id,
        None => {
            // The name of the repository and the name of the folder of the
            // subpath are names of a skill only when they are a valid id,
            // so the user is told to name the skill when they are not.
            let id = github_id(&options.source);
            if let Err(error) = validate_id(&id) {
                bail!(
                    "the id `{}` of the skill of the repository is not a valid id: {error}, use `--id` to name the skill",
                    catalog::redact(&id)
                );
            }
            id
        }
    };
    let checkout = fetch_github(&options.source, runner)?;
    let directory = match options.source.subpath.as_deref() {
        Some(subpath) => {
            // A repository can hold a symlink in the middle of the
            // subpath, and a symlink can point at anything, so every part
            // of the subpath is checked before the folder is opened.
            check_github_subpath_links(&checkout.staging.path, subpath)?;
            checkout.staging.path.join(subpath)
        }
        None => checkout.staging.path.clone(),
    };
    let from = github_provenance(&options.source, &checkout.sha);
    add_skill(&AddOptions {
        source: directory,
        store: options.store.clone(),
        id: Some(id),
        from: Some(from),
    })
}

/// The id of a skill of a repository of GitHub.
///
/// The id is the name of the folder of the subpath, else the name of the
/// repository, so two skills of two repositories that share a name stay two
/// skills.
fn github_id(source: &GitHubSource) -> String {
    match source.subpath.as_deref() {
        Some(subpath) => subpath
            .rsplit('/')
            .next()
            .unwrap_or(source.repo.as_str())
            .to_string(),
        None => source.repo.clone(),
    }
}

/// The provenance of a skill of a repository of GitHub.
///
/// The provenance names the owner, the repository, the commit that was
/// fetched and the subpath of the skill.
fn github_provenance(source: &GitHubSource, sha: &str) -> String {
    format!(
        "github:{}/{}@{}//{}",
        source.owner,
        source.repo,
        sha,
        source.subpath.as_deref().unwrap_or("")
    )
}

/// A repository of GitHub that was fetched into a temporary directory.
struct GitHubCheckout {
    /// The temporary directory that holds the fetched repository.
    staging: Staging,
    /// The commit that was fetched.
    sha: String,
}

/// Fetch a repository of GitHub into a fresh temporary directory.
///
/// The repository is fetched with the git command line, without a prompt for
/// a password, and the hooks of the repository are never run, because a
/// repository of somebody else must not run anything on the machine of the
/// user. The hooks are off for every command that can run one, so the hook
/// of a checkout that came with the templates of git never runs either.
///
/// The temporary directory goes away with the command, also when the fetch
/// fails.
fn fetch_github(source: &GitHubSource, runner: &dyn GitRunner) -> Result<GitHubCheckout> {
    let staging = Staging::named("skrun-github")?;
    let url = format!("https://github.com/{}/{}.git", source.owner, source.repo);
    // A source without a ref takes the default branch of the repository.
    let reference = source
        .reference
        .clone()
        .unwrap_or_else(|| "HEAD".to_string());
    git(runner, &git_args(&["init"]), &staging.path)?;
    git(
        runner,
        &git_args(&[
            "fetch",
            "--depth",
            "1",
            "--no-recurse-submodules",
            url.as_str(),
            reference.as_str(),
        ]),
        &staging.path,
    )?;
    git(
        runner,
        &git_args(&["checkout", "--detach", "FETCH_HEAD"]),
        &staging.path,
    )?;
    let sha = git(runner, &git_args(&["rev-parse", "HEAD"]), &staging.path)?
        .trim()
        .to_string();
    check_sha(&sha)?;
    Ok(GitHubCheckout { staging, sha })
}

/// The command line of one git command.
///
/// Every command of the fetch holds the options that hold back the
/// features of git that run code, so the hooks of the repository, the
/// helpers of the credentials, the submodules and the filters of the files
/// never run.
fn git_args(command: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = GIT_SAFE_OPTIONS
        .iter()
        .map(|option| option.to_string())
        .collect();
    for argument in command {
        args.push(argument.to_string());
    }
    args
}

/// Run one git command and fail when the command does not end well.
fn git(runner: &dyn GitRunner, args: &[String], directory: &Path) -> Result<String> {
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = runner.run(&borrowed, directory)?;
    if !output.success {
        let reason = output.stderr.trim();
        let reason = if reason.is_empty() {
            "the command failed".to_string()
        } else {
            reason.to_string()
        };
        bail!(
            "`git {}` failed: {}",
            catalog::redact(&args.join(" ")),
            catalog::redact(&reason)
        );
    }
    Ok(output.stdout)
}

/// Check the commit of a fetched repository.
///
/// The commit is the full name of the object, so a command line that
/// writes an abbreviation of a commit or something else never ends up in
/// the provenance of a skill.
fn check_sha(sha: &str) -> Result<()> {
    let full = sha.len() == 40 || sha.len() == 64;
    let hexadecimal = sha
        .chars()
        .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character));
    if !full || !hexadecimal {
        bail!("the fetched repository has no full commit id");
    }
    Ok(())
}

/// Copy a skill root into the store.
///
/// The copy is made beside its final place and then moved there, so a stored
/// version never appears half written and an existing version is never
/// overwritten.
fn store_copy(source: &Path, parent: &Path, target: &Path) -> Result<()> {
    let incoming = parent.join(format!(
        ".incoming-{}-{}-{}",
        file_name(target),
        std::process::id(),
        nanos()
    ));
    let _ = remove_tree(&incoming);
    let copied = copy_readonly(source, &incoming).and_then(|()| lock_directories(&incoming));
    if let Err(error) = copied {
        let _ = remove_tree(&incoming);
        return Err(error)
            .with_context(|| format!("store skill in `{}`", catalog::redact_path(target)));
    }
    if let Err(error) = fs::rename(&incoming, target) {
        let _ = remove_tree(&incoming);
        if target.exists() {
            bail!(
                "skill version `{}` is already stored",
                catalog::redact(&file_name(target))
            );
        }
        return Err(error)
            .with_context(|| format!("store skill in `{}`", catalog::redact_path(target)));
    }
    Ok(())
}

/// Remove a tree of files, even when its directories are read-only.
fn remove_tree(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let _ = unlock_directories(path);
    fs::remove_dir_all(path).with_context(|| format!("remove `{}`", catalog::redact_path(path)))
}

/// The threat record of one stored skill id.
///
/// An id that was never added has no record, so the command fails.
pub fn read_record(root: &Path, id: &str) -> Result<ThreatRecord> {
    if let Err(error) = validate_id(id) {
        bail!(
            "skill id `{}` is not a valid id: {error}",
            catalog::redact(id)
        );
    }
    let path = record_path(root, id);
    if !path.is_file() {
        bail!("no stored skill `{}`", catalog::redact(id));
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read record `{}`", catalog::redact_path(&path)))?;
    serde_json::from_str(&text)
        .with_context(|| format!("read record `{}`", catalog::redact_path(&path)))
}

/// Every stored skill id, with the hash and the findings of its newest
/// version.
pub fn list_records(root: &Path) -> Result<Vec<StoreSummary>> {
    let mut summaries = Vec::new();
    for record in read_records(root)? {
        let Some(version) = record.versions.last() else {
            continue;
        };
        summaries.push(StoreSummary {
            id: catalog::redact(&record.id),
            hash: catalog::redact(&version.hash),
            tier: version.tier,
            high: count_findings(version, Severity::High),
            medium: count_findings(version, Severity::Medium),
            low: count_findings(version, Severity::Low),
        });
    }
    Ok(summaries)
}

/// Every record of the store, sorted by the id of the skill.
///
/// A record that cannot be read and a record that cannot be parsed are
/// skipped, so a broken record never stops the listing of the store and
/// never stops the search of the store.
fn read_records(root: &Path) -> Result<Vec<ThreatRecord>> {
    let directory = root.join("records");
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for path in read_dir_sorted(&directory)? {
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<ThreatRecord>(&text) else {
            continue;
        };
        records.push(record);
    }
    records.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(records)
}

/// The number of findings of one severity in one version.
fn count_findings(version: &StoredVersion, severity: Severity) -> u64 {
    version
        .findings
        .iter()
        .filter(|finding| finding.severity == severity)
        .count() as u64
}

/// Redact every string value of a JSON value.
///
/// Every string of the value goes through the redaction helper of the
/// catalog, so a value that looks like a secret never reaches the output.
/// The keys of an object are the names of the fields of the record, so they
/// hold no user text.
pub fn redact_json_strings(value: &Value) -> Value {
    catalog::redact_json_strings(value)
}

/// The threat record of a skill id, with every string value redacted.
///
/// The record is parsed and every string of it goes through the redaction
/// helper of the catalog, so a value that looks like a secret never reaches
/// the output, not even a value of a record that was written by hand.
pub fn redacted_record(record: &ThreatRecord) -> ThreatRecord {
    let Ok(value) = serde_json::to_value(record) else {
        return record.clone();
    };
    serde_json::from_value(redact_json_strings(&value)).unwrap_or_else(|_| record.clone())
}

/// The threat record of a skill id as JSON, with every string value
/// redacted.
pub fn redacted_record_json(record: &ThreatRecord) -> Result<String> {
    let value = serde_json::to_value(record)?;
    let mut text = serde_json::to_string_pretty(&redact_json_strings(&value))?;
    text.push('\n');
    Ok(text)
}

/// Render a threat record as text.
///
/// Every printed value goes through the redaction helper of the catalog, so a
/// value that looks like a secret never reaches the output.
pub fn render_record(record: &ThreatRecord) -> String {
    let record = &redacted_record(record);
    let mut text = String::new();
    text.push_str(&format!("skill {}\n", catalog::redact(&record.id)));
    for version in &record.versions {
        let from = match &version.from {
            Some(value) => catalog::redact(value),
            None => "unknown".to_string(),
        };
        text.push_str(&format!(
            "  version {} from {} added {} files {} bytes {} tier {}\n",
            catalog::redact(&version.hash),
            from,
            version.added_at,
            version.files,
            version.bytes,
            catalog::redact(version.tier.as_str())
        ));
        if !version.tier_reasons.is_empty() {
            let reasons = version
                .tier_reasons
                .iter()
                .map(|reason| catalog::redact(reason))
                .collect::<Vec<_>>()
                .join(", ");
            text.push_str(&format!("    tier reasons: {reasons}\n"));
        }
        for script in &version.scripts {
            let imports = if script.imports.is_empty() {
                "-".to_string()
            } else {
                script
                    .imports
                    .iter()
                    .map(|item| catalog::redact(item))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            text.push_str(&format!(
                "    script {} {} imports: {}\n",
                catalog::redact(&script.path),
                script.language,
                imports
            ));
        }
        for severity in [Severity::High, Severity::Medium, Severity::Low] {
            let group: Vec<&Finding> = version
                .findings
                .iter()
                .filter(|finding| finding.severity == severity)
                .collect();
            if group.is_empty() {
                continue;
            }
            text.push_str(&format!("    {} findings:\n", severity_name(severity)));
            for finding in group {
                let location = match finding.line {
                    Some(line) => format!("{}:{}", catalog::redact(&finding.path), line),
                    None => catalog::redact(&finding.path),
                };
                // The detail of a finding says what the rule found, so the
                // name of a module that a file hides is never lost.
                let detail = match &finding.detail {
                    Some(detail) => format!(" ({})", catalog::redact(detail)),
                    None => String::new(),
                };
                text.push_str(&format!(
                    "      finding {} {} {}{}\n",
                    severity_name(finding.severity),
                    finding.rule,
                    location,
                    detail
                ));
            }
        }
    }
    text
}

/// Render the list of stored skills as text.
pub fn render_summaries(summaries: &[StoreSummary]) -> String {
    let mut text = String::new();
    for summary in summaries {
        text.push_str(&format!(
            "{}\t{}\ttier {}\thigh {}\tmedium {}\tlow {}\n",
            catalog::redact(&summary.id),
            catalog::redact(&summary.hash),
            catalog::redact(summary.tier.as_str()),
            summary.high,
            summary.medium,
            summary.low
        ));
    }
    text
}

/// The options of a search of the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSearchOptions {
    /// The raw query; every whitespace separated term has to match.
    pub query: String,
    /// The highest tier that a version may have; every tier when it is
    /// `None`.
    pub max_tier: Option<Tier>,
    /// Whether only the versions that are safe to run are kept.
    pub safe: bool,
    /// The number of results to keep.
    pub limit: usize,
}

impl StoreSearchOptions {
    /// The options of a search for `query` without any filter.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            max_tier: None,
            safe: false,
            limit: catalog::SEARCH_LIMIT,
        }
    }
}

/// One stored skill that matches a search of the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoreSearchHit {
    /// The id of the skill.
    pub id: String,
    /// The score of the match; a higher score is a better match.
    pub score: u32,
    /// The name of the skill, from its frontmatter.
    pub title: Option<String>,
    /// The summary of the skill; the first sentence of the description. A
    /// summary that holds a secret is redacted, so a secret is never
    /// printed in part.
    pub summary: Option<String>,
    /// The tier that the newest version suggests.
    pub tier: Tier,
    /// The hash that names the newest version.
    pub hash: String,
    /// The number of high findings of the newest version.
    pub high: u64,
}

/// Search the newest version of every stored skill for the terms of
/// `options`.
///
/// Every term of the query has to occur in at least one field of a skill, so
/// the terms narrow the result down together. A term that occurs in several
/// fields adds the weight of every field, so a skill that matches on its id
/// outranks a skill that only mentions the term in its body.
///
/// The fields of a stored skill are its id, the name of its frontmatter, its
/// description, its tags and the body of its `SKILL.md`. A stored skill has
/// no registry, so the registry of a skills directory is never searched.
///
/// The newest version of a skill is the one that is searched, so a search
/// shows what the store holds today and not what it held before.
///
/// The result is sorted by score and then by id, so the same store always
/// gives the same result.
pub fn search_store(root: &Path, options: &StoreSearchOptions) -> Result<Vec<StoreSearchHit>> {
    let terms = catalog::search_terms(&options.query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let mut hits = Vec::new();
    for record in read_records(root)? {
        let Some(version) = record.versions.last() else {
            continue;
        };
        // A version of a tier that needs more room than the user allows is
        // never a result. A skill without a tier is guidance, so it always
        // passes the filter of the tier.
        if let Some(max) = options.max_tier
            && !version.tier.at_most(max)
        {
            continue;
        }
        let high = count_findings(version, Severity::High);
        // A safe version is a version that needs no container and no
        // microVM and that has no finding that must not run at all. The
        // tiers `none`, `L0` and `L1` are the only ones that are safe, so
        // a version of the tier `L2` or `L3` is never a result of a safe
        // search, not even when it holds no high finding at all.
        if options.safe && (high > 0 || !version.tier.at_most(Tier::L1)) {
            continue;
        }

        let text = stored_text(root, &record.id, version);
        let fields = store_fields(&record.id, &text);
        let Some(score) = catalog::score_fields(&fields, &terms) else {
            continue;
        };

        let summary = text
            .description
            .as_deref()
            .map(|description| catalog::first_sentence(description, catalog::SEARCH_SUMMARY_LIMIT))
            .filter(|sentence| !sentence.is_empty())
            .map(|sentence| catalog::redact(&sentence));

        hits.push(StoreSearchHit {
            id: record.id.clone(),
            score,
            title: text.name,
            summary,
            tier: version.tier,
            hash: version.hash.clone(),
            high,
        });
    }

    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    hits.truncate(options.limit);
    Ok(hits)
}

/// The text of a stored skill that a search reads.
struct StoredText {
    /// The name of the skill, from its frontmatter.
    name: Option<String>,
    /// The description of the skill, from its frontmatter.
    description: Option<String>,
    /// The tags of the skill, from its frontmatter.
    tags: Vec<String>,
    /// The text of the entrypoint of the skill.
    body: String,
}

/// The text of the newest version of a stored skill.
///
/// The frontmatter of the stored entrypoint is the frontmatter of the skill
/// today. A version whose entrypoint cannot be read keeps the name and the
/// description of its record, so a search still finds the skill by its id
/// and by its name.
fn stored_text(root: &Path, id: &str, version: &StoredVersion) -> StoredText {
    let body = stored_entrypoint(root, id, version).unwrap_or_default();
    let frontmatter = catalog::parse_frontmatter(&body);
    StoredText {
        name: frontmatter.name.or_else(|| version.name.clone()),
        description: frontmatter
            .description
            .or_else(|| version.description.clone()),
        tags: frontmatter.tags,
        body,
    }
}

/// The text of the entrypoint of a stored version.
///
/// The file is read through the folder of the version and without following
/// a symlink, so a file that is not part of the store is never read.
///
/// The file is read with the limit of a search and not with the limit of
/// `catalog cat`, so a big `SKILL.md` is still searched and its tags and
/// its body still match a term.
fn stored_entrypoint(root: &Path, id: &str, version: &StoredVersion) -> Option<String> {
    if validate_id(id).is_err() {
        return None;
    }
    let location = VersionLocation {
        id: id.to_string(),
        hash: version.hash.clone(),
        path: absolute_path(&root.join("skills").join(id).join(&version.hash)),
        root: absolute_path(root),
        version: version.clone(),
    };
    read_version_file_for_search(&location, SKILL_FILE).ok()
}

/// The fields of a stored skill that a search term can match.
///
/// Every field is lowercased once, so the terms only have to be lowercased
/// once as well.
fn store_fields(id: &str, text: &StoredText) -> Vec<(String, u32)> {
    let mut fields: Vec<(String, u32)> = Vec::new();
    fields.push((id.to_lowercase(), catalog::SEARCH_WEIGHT_ID));
    if let Some(name) = &text.name {
        fields.push((name.to_lowercase(), catalog::SEARCH_WEIGHT_NAME));
    }
    if let Some(description) = &text.description {
        fields.push((
            description.to_lowercase(),
            catalog::SEARCH_WEIGHT_DESCRIPTION,
        ));
    }
    for tag in &text.tags {
        fields.push((tag.to_lowercase(), catalog::SEARCH_WEIGHT_TAG));
    }
    fields.push((text.body.to_lowercase(), catalog::SEARCH_WEIGHT_BODY));
    fields
}

/// Render the hits of a search of the store as text.
///
/// Every printed value goes through the redaction helper, so a value that
/// looks like a secret never reaches the output.
pub fn render_store_search_text(hits: &[StoreSearchHit]) -> String {
    if hits.is_empty() {
        return "no skills match\n".to_string();
    }

    let mut out = String::new();
    for hit in hits {
        let label = hit.title.clone().unwrap_or_else(|| hit.id.clone());
        out.push_str(&format!(
            "{}  {}  {}  {}\n",
            catalog::redact(&hit.id),
            hit.tier.as_str(),
            catalog::redact(&hit.hash),
            catalog::redact(&label)
        ));
        if let Some(summary) = &hit.summary {
            out.push_str(&format!("  {}\n", catalog::redact(summary)));
        }
    }
    out
}

/// Render the hits of a search of the store as JSON.
///
/// The values go through the same redaction helper as the text output.
pub fn render_store_search_json(hits: &[StoreSearchHit]) -> Result<String> {
    let redacted: Vec<StoreSearchHit> = hits.iter().map(redacted_store_hit).collect();
    let mut text = serde_json::to_string_pretty(&redacted)?;
    text.push('\n');
    Ok(text)
}

/// A search hit of the store with every value that goes through the
/// redaction helper.
fn redacted_store_hit(hit: &StoreSearchHit) -> StoreSearchHit {
    StoreSearchHit {
        id: catalog::redact(&hit.id),
        score: hit.score,
        title: hit.title.as_deref().map(catalog::redact),
        summary: hit.summary.as_deref().map(catalog::redact),
        tier: hit.tier,
        hash: catalog::redact(&hit.hash),
        high: hit.high,
    }
}

/// The largest file that the `catalog cat` command prints.
///
/// A file of a stored skill is read as a whole, so a file that is bigger
/// than this is refused instead of filling the memory of the command.
pub const MAX_CAT_BYTES: u64 = 2 * 1000 * 1000;

/// The largest file that a search of the store reads.
///
/// The limit of `catalog cat` is not the limit of a search: a search only
/// looks for a term in the text of a skill and never prints that text, so
/// a big `SKILL.md` is still searched. The limit keeps a huge file from
/// filling the memory of the command, so the text of a file that is bigger
/// than this is cut off instead of being read as a whole.
pub const MAX_SEARCH_BYTES: u64 = 16 * 1000 * 1000;

/// Where a stored version of a skill lives.
///
/// The folder of a version never changes, so the path of a version is the
/// place where the files of that version can be looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionLocation {
    /// The id of the skill.
    pub id: String,
    /// The hash that names the version.
    pub hash: String,
    /// The absolute path of the folder of the version.
    pub path: PathBuf,
    /// The absolute path of the root of the store.
    ///
    /// The root is the folder that the user named, so it is the one place
    /// of the store that is trusted: every folder below it is opened below
    /// the folder before it and without following a symlink.
    pub root: PathBuf,
    /// The record of the version.
    pub version: StoredVersion,
}

/// The folder of a stored version of a skill.
///
/// The newest version of the skill is taken when no version is named. A
/// version is named by the hash of the record, by the whole content hash or
/// by the start of the content hash, when that start names one version and
/// no other.
///
/// A skill that was never added and a version that was never stored have no
/// folder, so the command fails.
pub fn version_location(root: &Path, id: &str, version: Option<&str>) -> Result<VersionLocation> {
    let record = read_record(root, id)?;
    let stored = select_version(&record, version)?;
    let hash = stored.hash.clone();
    let folder = root.join("skills").join(id).join(&hash);
    Ok(VersionLocation {
        id: id.to_string(),
        hash,
        path: absolute_path(&folder),
        root: absolute_path(root),
        version: stored.clone(),
    })
}

/// The folder of a stored version, when the store holds it.
///
/// A version of a record whose folder is gone is not a version of the
/// store, so a file of it cannot be read and its folder cannot be shown.
///
/// The folder is opened below the root of the store and without following a
/// symlink, so a folder of the store that is a link is never taken for the
/// folder of a version.
pub fn stored_folder(location: &VersionLocation) -> Result<PathBuf> {
    verify_version_folder(location)?;
    Ok(location.path.clone())
}

/// Check that the folder of a version is a folder of the store.
///
/// The folder is opened below the root of the store and without following a
/// symlink, so a folder of the store that is a link is refused.
#[cfg(unix)]
fn verify_version_folder(location: &VersionLocation) -> Result<()> {
    let _folder = open_version_dir(location)?;
    Ok(())
}

/// Check that the folder of a version is a folder of the store.
///
/// Without a system of unix the folder is looked at where it is, so a
/// folder that is a link is refused.
#[cfg(not(unix))]
fn verify_version_folder(location: &VersionLocation) -> Result<()> {
    let metadata = match fs::symlink_metadata(&location.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "the stored version `{}` of the skill `{}` is not in the store",
                catalog::redact(&location.hash),
                catalog::redact(&location.id)
            );
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)
                .context(format!("read `{}`", catalog::redact_path(&location.path))));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "the stored version `{}` of the skill `{}` is not in the store",
            catalog::redact(&location.hash),
            catalog::redact(&location.id)
        );
    }
    Ok(())
}

/// The version of a record that is asked for.
///
/// The newest version is the last one of the record, because a version is
/// added to the end of the record when it is stored.
fn select_version<'a>(record: &'a ThreatRecord, wanted: Option<&str>) -> Result<&'a StoredVersion> {
    let Some(wanted) = wanted else {
        return record.versions.last().ok_or_else(|| {
            anyhow::anyhow!(
                "the skill `{}` has no stored version",
                catalog::redact(&record.id)
            )
        });
    };
    let wanted = wanted.trim();
    if let Some(version) = record
        .versions
        .iter()
        .find(|version| version.hash == wanted)
    {
        return Ok(version);
    }
    if let Some(version) = record
        .versions
        .iter()
        .find(|version| version.full_hash == wanted)
    {
        return Ok(version);
    }
    if wanted.len() >= 4 {
        let mut found = record
            .versions
            .iter()
            .filter(|version| version.full_hash.starts_with(wanted));
        if let Some(version) = found.next()
            && found.next().is_none()
        {
            return Ok(version);
        }
    }

    bail!(
        "the version `{}` of the skill `{}` is not stored",
        catalog::redact(wanted),
        catalog::redact(&record.id)
    )
}

/// The text of a file of a stored version.
///
/// The file is named by a path that is relative to the folder of the
/// version. A path that leaves the folder of the version, a path that is
/// absolute and a path that holds a symlink are refused, so a file of the
/// store is the only thing that is ever read.
///
/// The file is opened through the folder of the version and without
/// following a symlink, and the type and the size of the file that was
/// opened are the ones that are checked. A file that is put in the place
/// of the file while the command runs is therefore never read.
///
/// A file that is not text and a file that is bigger than `MAX_CAT_BYTES`
/// are refused as well: the command prints a file, not a part of it.
pub fn read_version_file(location: &VersionLocation, relative: &str) -> Result<String> {
    let relative = safe_relative(relative)?;
    let path = location.path.join(&relative);
    let file = open_version_file(location, &relative)?;
    let metadata = file
        .metadata()
        .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
    if !metadata.is_file() {
        bail!(
            "the file `{}` is not a file of the stored version `{}`",
            catalog::redact(&relative.to_string_lossy()),
            catalog::redact(&location.hash)
        );
    }
    if metadata.len() > MAX_CAT_BYTES {
        bail!(
            "the file `{}` is bigger than {MAX_CAT_BYTES} bytes",
            catalog::redact(&relative.to_string_lossy())
        );
    }
    let mut bytes = Vec::new();
    // The file is read with a limit, so a file that is bigger than the
    // limit is refused even when it grew after it was opened.
    file.take(MAX_CAT_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
    if bytes.len() as u64 > MAX_CAT_BYTES {
        bail!(
            "the file `{}` is bigger than {MAX_CAT_BYTES} bytes",
            catalog::redact(&relative.to_string_lossy())
        );
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        anyhow::anyhow!(
            "the file `{}` is not valid UTF-8",
            catalog::redact(&relative.to_string_lossy())
        )
    })?;
    Ok(text)
}

/// The text of a file of a stored version, for a search of the store.
///
/// The file is opened like the file of `catalog cat`, so a symlink is
/// never followed and a file outside of the store is never read. The
/// limit of `catalog cat` is not used here: a search matches a term in
/// the text of a skill and never prints that text, so a big `SKILL.md` is
/// still searched and its tags and its body still match.
///
/// A file that is bigger than `MAX_SEARCH_BYTES` is cut off instead of
/// being refused, so a big file never fills the memory of the command and
/// never silences every term of the file.
fn read_version_file_for_search(location: &VersionLocation, relative: &str) -> Result<String> {
    let relative = safe_relative(relative)?;
    let path = location.path.join(&relative);
    let file = open_version_file(location, &relative)?;
    let metadata = file
        .metadata()
        .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
    if !metadata.is_file() {
        bail!(
            "the file `{}` is not a file of the stored version `{}`",
            catalog::redact(&relative.to_string_lossy()),
            catalog::redact(&location.hash)
        );
    }
    let mut bytes = Vec::new();
    // The file is read with a limit, so a file that grew after it was
    // opened never fills the memory of the command.
    file.take(MAX_SEARCH_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
    if bytes.len() as u64 > MAX_SEARCH_BYTES {
        bytes.truncate(MAX_SEARCH_BYTES as usize);
    }
    // The bytes that are not text are not searched, but the text that can
    // be read is kept, so a term in it still matches.
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The folders of the store that hold a version of a skill.
///
/// Every name is a single part of a path, so a name that holds a separator
/// names no folder of the store and never a folder outside of it.
fn store_parts(location: &VersionLocation) -> Result<Vec<String>> {
    let parts = [
        "skills".to_string(),
        location.id.clone(),
        location.hash.clone(),
    ];
    for part in &parts {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains('/')
            || part.contains('\0')
        {
            bail!(
                "the name `{}` is not a name of the store",
                catalog::redact(part)
            );
        }
    }
    Ok(parts.to_vec())
}

/// Open a file of a stored version without following a symlink.
///
/// The root of the store is opened first, and every folder of the path of
/// the file is opened below the folder before it and without following a
/// symlink. A folder of the store or a folder of a version that is put in
/// the place of another one while the command runs is therefore never read
/// through.
#[cfg(unix)]
fn open_version_file(location: &VersionLocation, relative: &Path) -> Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
    use std::os::unix::ffi::OsStrExt;

    let mut names = Vec::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            bail!(
                "the file `{}` must be a path inside the stored version",
                catalog::redact(&relative.to_string_lossy())
            );
        };
        let name = std::ffi::CString::new(part.as_bytes()).map_err(|_| {
            anyhow::anyhow!(
                "the file `{}` must be a path inside the stored version",
                catalog::redact(&relative.to_string_lossy())
            )
        })?;
        names.push(name);
    }
    let Some((last, parents)) = names.split_last() else {
        bail!(
            "the file `{}` names no file",
            catalog::redact(&relative.to_string_lossy())
        );
    };
    let mut directory = open_version_dir(location)?;
    for name in parents {
        directory = open_relative(directory.as_raw_fd(), name, true, location, relative)?;
    }
    let file = open_relative(directory.as_raw_fd(), last, false, location, relative)?;
    Ok(unsafe { fs::File::from_raw_fd(file.into_raw_fd()) })
}

/// Open a file of a stored version without following a reparse point.
///
/// Windows cannot open a file below a folder without following a reparse
/// point, so every part of the path is opened with a handle that does not
/// follow a reparse point and every handle is looked at. A reparse point
/// in the place of a part of the path is refused, so a file of another
/// folder is never read.
#[cfg(windows)]
fn open_version_file(location: &VersionLocation, relative: &Path) -> Result<fs::File> {
    let mut path = location.root.clone();
    for part in store_parts(location)? {
        path.push(&part);
        let folder = open_no_reparse(&path, true)?;
        let metadata = folder
            .metadata()
            .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
        if metadata.file_type().is_symlink() {
            bail!(
                "the folder `{}` of the store is a symlink, a symlink is never followed",
                catalog::redact(&part)
            );
        }
        if !metadata.is_dir() {
            bail!(
                "the stored version `{}` of the skill `{}` is not in the store",
                catalog::redact(&location.hash),
                catalog::redact(&location.id)
            );
        }
    }

    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            bail!(
                "the file `{}` must be a path inside the stored version",
                catalog::redact(&relative.to_string_lossy())
            );
        };
        parts.push(part.to_os_string());
    }
    let Some((last, parents)) = parts.split_last() else {
        bail!(
            "the file `{}` names no file",
            catalog::redact(&relative.to_string_lossy())
        );
    };
    for part in parents {
        path.push(part);
        let folder = open_no_reparse(&path, true)?;
        let metadata = folder
            .metadata()
            .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
        if metadata.file_type().is_symlink() {
            bail!(
                "the file `{}` is a symlink, a symlink is never followed",
                catalog::redact(&relative.to_string_lossy())
            );
        }
        if !metadata.is_dir() {
            bail!(
                "the file `{}` is not a file of the stored version `{}`",
                catalog::redact(&relative.to_string_lossy()),
                catalog::redact(&location.hash)
            );
        }
    }
    path.push(last);
    let file = open_no_reparse(&path, false)?;
    let metadata = file
        .metadata()
        .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "the file `{}` is a symlink, a symlink is never followed",
            catalog::redact(&relative.to_string_lossy())
        );
    }
    if !metadata.is_file() {
        bail!(
            "the file `{}` is not a file of the stored version `{}`",
            catalog::redact(&relative.to_string_lossy()),
            catalog::redact(&location.hash)
        );
    }
    // The file that was opened is inside the version, so a folder of the
    // path that is put in the place of a link while the command runs never
    // shows a file of another folder.
    stay_inside(&location.path, &path)?;
    Ok(file)
}

/// Open a path without following a reparse point.
///
/// The handle of a reparse point is the handle of the reparse point itself,
/// so a link in the place of the path is seen and refused.
#[cfg(windows)]
fn open_no_reparse(path: &Path, want_directory: bool) -> Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    /// Open the reparse point itself and not the file that it points at.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    /// Open a folder, which cannot be opened without this flag.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
    if want_directory {
        flags |= FILE_FLAG_BACKUP_SEMANTICS;
    }
    let mut options = fs::OpenOptions::new();
    options.custom_flags(flags);
    if !want_directory {
        options.read(true);
    }
    options
        .open(path)
        .with_context(|| format!("read `{}`", catalog::redact_path(path)))
}

/// Open a file of a stored version without following a symlink.
///
/// Without a system of unix a file cannot be opened below a folder without
/// following a symlink, so the file is checked before it is opened and the
/// file that was opened is checked again.
#[cfg(not(any(unix, windows)))]
fn open_version_file(location: &VersionLocation, relative: &Path) -> Result<fs::File> {
    // The names of the folders of the store are checked, so a name that is
    // not a name of the store is refused.
    let _parts = store_parts(location)?;
    let path = location.path.join(relative);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "the file `{}` is not in the stored version `{}`",
                catalog::redact(&relative.to_string_lossy()),
                catalog::redact(&location.hash)
            );
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)
                .context(format!("read `{}`", catalog::redact_path(&path))));
        }
    };
    if !metadata.is_file() {
        bail!(
            "the file `{}` is not a file of the stored version `{}`",
            catalog::redact(&relative.to_string_lossy()),
            catalog::redact(&location.hash)
        );
    }
    let file =
        fs::File::open(&path).with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
    let opened = file
        .metadata()
        .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
    if !opened.is_file() || opened.len() != metadata.len() {
        bail!(
            "the file `{}` is not a file of the stored version `{}`",
            catalog::redact(&relative.to_string_lossy()),
            catalog::redact(&location.hash)
        );
    }
    Ok(file)
}

/// The folder of a stored version, opened without following a symlink.
///
/// The root of the store is the folder that the user named, so it is opened
/// as it is named. Every folder below it is opened below the folder before
/// it and without following a symlink, so a folder of the store that is
/// put in the place of another one is never read through.
#[cfg(unix)]
fn open_version_dir(location: &VersionLocation) -> Result<std::os::fd::OwnedFd> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    let root = std::ffi::CString::new(location.root.as_os_str().as_bytes()).map_err(|_| {
        anyhow::anyhow!(
            "the store of the skill `{}` has no name",
            catalog::redact(&location.id)
        )
    })?;
    // The root is the folder that the user named, so it is opened as it is
    // named; only the folders below it are refused when they are a link.
    let fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(anyhow::Error::new(std::io::Error::last_os_error())
            .context(format!("read `{}`", catalog::redact_path(&location.root))));
    }
    let mut directory = unsafe { OwnedFd::from_raw_fd(fd) };
    for part in store_parts(location)? {
        directory = open_store_folder(directory.as_raw_fd(), &part, location)?;
    }
    Ok(directory)
}

/// Open a folder of the store without following a symlink.
///
/// The folder is opened below the folder before it, so the name of the
/// folder never names another folder of the store.
#[cfg(unix)]
fn open_store_folder(
    directory: std::os::fd::RawFd,
    name: &str,
    location: &VersionLocation,
) -> Result<std::os::fd::OwnedFd> {
    use std::os::fd::{FromRawFd, OwnedFd};

    let name = std::ffi::CString::new(name).map_err(|_| {
        anyhow::anyhow!(
            "the folder `{}` of the store has no name",
            catalog::redact(name)
        )
    })?;
    let fd = unsafe {
        libc::openat(
            directory,
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_DIRECTORY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(store_open_error(
            std::io::Error::last_os_error(),
            &name,
            directory,
            location,
        ));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The error of a folder of the store that cannot be opened.
///
/// A symlink is never followed, so a link is named as a link and not as a
/// folder that cannot be read.
#[cfg(unix)]
fn store_open_error(
    error: std::io::Error,
    name: &std::ffi::CStr,
    directory: std::os::fd::RawFd,
    location: &VersionLocation,
) -> anyhow::Error {
    let label = catalog::redact(&name.to_string_lossy());
    match error.raw_os_error() {
        Some(libc::ELOOP) => anyhow::anyhow!(
            "the folder `{}` of the store is a symlink, a symlink is never followed",
            label
        ),
        Some(libc::ENOTDIR) if is_symlink_at(directory, name) => anyhow::anyhow!(
            "the folder `{}` of the store is a symlink, a symlink is never followed",
            label
        ),
        Some(libc::ENOENT) | Some(libc::ENOTDIR) => anyhow::anyhow!(
            "the stored version `{}` of the skill `{}` is not in the store",
            catalog::redact(&location.hash),
            catalog::redact(&location.id)
        ),
        Some(libc::EACCES) | Some(libc::EPERM) => {
            anyhow::anyhow!("the folder `{}` of the store cannot be read", label)
        }
        _ => anyhow::Error::new(error).context(format!("read `{}`", label)),
    }
}

/// Whether a name below a folder is a symlink.
///
/// The name is looked at without following a symlink, so the answer is
/// about the name itself and not about the file that it points at.
#[cfg(unix)]
fn is_symlink_at(directory: std::os::fd::RawFd, name: &std::ffi::CStr) -> bool {
    let mut status: libc::stat = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::fstatat(
            directory,
            name.as_ptr(),
            &mut status,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    result == 0 && status.st_mode as u32 & libc::S_IFMT as u32 == libc::S_IFLNK as u32
}

/// Open a name below a folder, without following a symlink.
///
/// The descriptor of the folder is the one of the version, so the name is
/// never resolved through another folder.
#[cfg(unix)]
fn open_relative(
    directory: std::os::fd::RawFd,
    name: &std::ffi::CStr,
    want_directory: bool,
    location: &VersionLocation,
    relative: &Path,
) -> Result<std::os::fd::OwnedFd> {
    openat_raw(directory, name, want_directory)
        .map_err(|error| open_error(error, location, relative, directory, name))
}

/// Open a name below a folder, without following a symlink.
///
/// The name is opened below the descriptor of the folder, so the name never
/// names a file of another folder.
#[cfg(unix)]
fn openat_raw(
    directory: std::os::fd::RawFd,
    name: &std::ffi::CStr,
    want_directory: bool,
) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::{FromRawFd, OwnedFd};

    let mut flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    if want_directory {
        flags |= libc::O_DIRECTORY;
    }
    let fd = unsafe { libc::openat(directory, name.as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The error of a file of a version that cannot be opened.
///
/// A symlink is never followed, so a link is named as a link and not as a
/// file that cannot be read.
#[cfg(unix)]
fn open_error(
    error: std::io::Error,
    location: &VersionLocation,
    relative: &Path,
    directory: std::os::fd::RawFd,
    name: &std::ffi::CStr,
) -> anyhow::Error {
    let label = catalog::redact(&relative.to_string_lossy());
    match error.raw_os_error() {
        Some(libc::ELOOP) => {
            anyhow::anyhow!(
                "the file `{}` is a symlink, a symlink is never followed",
                label
            )
        }
        Some(libc::ENOTDIR) if is_symlink_at(directory, name) => {
            anyhow::anyhow!(
                "the file `{}` is a symlink, a symlink is never followed",
                label
            )
        }
        Some(libc::ENOENT) => anyhow::anyhow!(
            "the file `{}` is not in the stored version `{}`",
            label,
            catalog::redact(&location.hash)
        ),
        Some(libc::ENOTDIR) => anyhow::anyhow!(
            "the file `{}` is not a file of the stored version `{}`",
            label,
            catalog::redact(&location.hash)
        ),
        Some(libc::EACCES) | Some(libc::EPERM) => {
            anyhow::anyhow!("the file `{}` cannot be read", label)
        }
        _ => anyhow::Error::new(error).context(format!("read `{}`", label)),
    }
}

/// The markdown files of a stored version.
///
/// The paths are relative to the folder of the version and sorted, so the
/// list of a version is the same on every machine.
pub fn list_version_markdown(location: &VersionLocation) -> Result<Vec<String>> {
    let mut files = Vec::new();
    collect_markdown(location, &mut files)?;
    files.sort();
    Ok(files)
}

/// The markdown files of a stored version.
///
/// The folders are read through their descriptor, so a folder that is put
/// in the place of another folder while the command runs never shows a file
/// of another folder.
#[cfg(unix)]
fn collect_markdown(location: &VersionLocation, out: &mut Vec<String>) -> Result<()> {
    use std::os::fd::AsRawFd;

    let directory = open_version_dir(location)?;
    collect_markdown_at(directory.as_raw_fd(), "", out, location)
}

/// The markdown files below a folder, read through its descriptor.
///
/// Every entry is looked at without following a symlink, and every folder
/// is opened below the folder before it and without following a symlink, so
/// a link is never followed and never listed.
#[cfg(unix)]
fn collect_markdown_at(
    directory: std::os::fd::RawFd,
    prefix: &str,
    out: &mut Vec<String>,
    location: &VersionLocation,
) -> Result<()> {
    use std::os::fd::AsRawFd;

    for (name, kind) in read_dir_at(directory)? {
        let Some(name) = name.to_str() else {
            bail!(
                "an entry of the stored version `{}` has a name that is not valid UTF-8",
                catalog::redact(&location.hash)
            );
        };
        let relative = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        if kind == libc::S_IFLNK as u32 {
            // A link is never followed and never listed.
            continue;
        }
        if kind == libc::S_IFDIR as u32 {
            let name = std::ffi::CString::new(name).map_err(|_| {
                anyhow::anyhow!(
                    "the file `{}` must be a path inside the stored version",
                    catalog::redact(&relative)
                )
            })?;
            match openat_raw(directory, &name, true) {
                Ok(child) => {
                    collect_markdown_at(child.as_raw_fd(), &relative, out, location)?;
                }
                Err(error) if is_gone_error(&error) => {
                    // The folder is a link now or it is gone, so it is not
                    // read and it is not listed.
                    continue;
                }
                Err(error) => {
                    return Err(open_error(
                        error,
                        location,
                        Path::new(&relative),
                        directory,
                        &name,
                    ));
                }
            }
        } else if kind == libc::S_IFREG as u32 && is_markdown(&relative) {
            out.push(relative);
        }
    }
    Ok(())
}

/// Whether an error of an open is the error of a name that is gone.
///
/// A name that is a link now, a name that is gone and a name that is not a
/// folder are all names that hold no folder of the version, so they are
/// never read and never listed.
#[cfg(unix)]
fn is_gone_error(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ELOOP) | Some(libc::ENOENT) | Some(libc::ENOTDIR)
    )
}

/// The entries of a folder, read through its descriptor.
///
/// Every entry is looked at without following a symlink, so the type of an
/// entry is the type of the entry itself and not the type of the file that
/// a link points at.
#[cfg(unix)]
fn read_dir_at(directory: std::os::fd::RawFd) -> Result<Vec<(std::ffi::OsString, u32)>> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStringExt;

    // The folder is read through a copy of its descriptor, so the
    // descriptor of the caller stays open.
    let copy = unsafe { libc::dup(directory) };
    if copy < 0 {
        return Err(anyhow::Error::new(std::io::Error::last_os_error())
            .context("read the folder of the stored version"));
    }
    let handle = unsafe { libc::fdopendir(copy) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(copy) };
        return Err(anyhow::Error::new(error).context("read the folder of the stored version"));
    }
    let mut entries = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(handle) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        let bytes = name.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let mut status: libc::stat = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::fstatat(
                directory,
                name.as_ptr(),
                &mut status,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                // The entry is gone, so it is not listed.
                continue;
            }
            unsafe { libc::closedir(handle) };
            return Err(anyhow::Error::new(error).context("read the folder of the stored version"));
        }
        let kind = status.st_mode as u32 & libc::S_IFMT as u32;
        entries.push((std::ffi::OsString::from_vec(bytes.to_vec()), kind));
    }
    unsafe { libc::closedir(handle) };
    Ok(entries)
}

/// The markdown files of a stored version.
///
/// Every folder is opened with a handle that does not follow a reparse
/// point, so a folder that is a link is never read and never listed.
#[cfg(windows)]
fn collect_markdown(location: &VersionLocation, out: &mut Vec<String>) -> Result<()> {
    let folder = stored_folder(location)?;
    let mut files = Vec::new();
    collect_markdown_below(&folder, &folder, "", &mut files)?;
    out.extend(files);
    Ok(())
}

/// The markdown files of a stored version.
///
/// Every folder is looked at before it is read, so a folder that is a link
/// is never read and never listed.
#[cfg(not(any(unix, windows)))]
fn collect_markdown(location: &VersionLocation, out: &mut Vec<String>) -> Result<()> {
    let folder = stored_folder(location)?;
    let mut files = Vec::new();
    collect_markdown_below(&folder, &folder, "", &mut files)?;
    out.extend(files);
    Ok(())
}

/// The markdown files below a folder, relative to that folder.
///
/// A symlink is never followed and never listed, because a symlink can
/// point at anything. A folder that is not inside the version is never
/// read, so the list holds no file of another folder.
#[cfg(not(unix))]
fn collect_markdown_below(
    root: &Path,
    dir: &Path,
    prefix: &str,
    out: &mut Vec<String>,
) -> Result<()> {
    for path in read_dir_sorted(dir)? {
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let name = utf8_name(&path, "entry")?;
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if metadata.is_dir() {
            stay_inside(root, &path)?;
            #[cfg(windows)]
            {
                // The folder is opened with a handle that does not follow a
                // reparse point, so a folder that is a link is not read.
                let handle = open_no_reparse(&path, true)?;
                let opened = handle
                    .metadata()
                    .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
                if opened.file_type().is_symlink() || !opened.is_dir() {
                    continue;
                }
            }
            collect_markdown_below(root, &path, &relative, out)?;
        } else if metadata.is_file() && is_markdown(&relative) {
            out.push(relative);
        }
    }
    Ok(())
}

/// Whether a path names a markdown file.
fn is_markdown(relative: &str) -> bool {
    Path::new(relative)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// The path of a file inside the folder of a version.
///
/// Only a relative path of normal names is accepted. A path that is
/// absolute and a path that holds `..` would leave the folder of the
/// version, so both are refused.
fn safe_relative(relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute() {
        bail!(
            "the file `{}` must be a path inside the stored version",
            catalog::redact(relative)
        );
    }
    let mut inside = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => inside.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                bail!(
                    "the file `{}` must not leave the stored version",
                    catalog::redact(relative)
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!(
                    "the file `{}` must be a path inside the stored version",
                    catalog::redact(relative)
                );
            }
        }
    }
    if inside.as_os_str().is_empty() {
        bail!("the file `{}` names no file", catalog::redact(relative));
    }
    Ok(inside)
}

/// Refuse a file that is not inside the folder of the version.
///
/// The folder of the version and the file are resolved before they are
/// compared, so a file that leaves the folder through a link is refused.
#[cfg(not(unix))]
fn stay_inside(dir: &Path, file: &Path) -> Result<()> {
    let (Ok(real_dir), Ok(real_file)) = (fs::canonicalize(dir), fs::canonicalize(file)) else {
        return Ok(());
    };
    if !real_file.starts_with(&real_dir) {
        bail!(
            "the file `{}` is not inside the stored version",
            catalog::redact_path(file)
        );
    }
    Ok(())
}

/// The absolute path of a path.
///
/// The path is not resolved through symlinks, so the path of a store stays
/// the path that the user named.
fn absolute_path(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(current) => current.join(path),
            Err(_) => path.to_path_buf(),
        }
    };
    let mut absolute = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            other => absolute.push(other.as_os_str()),
        }
    }
    absolute
}

/// The error of a version that is not safe to read.
///
/// The command that reads a version with `--require-safe` ends with its own
/// status, so the user sees that the version is not safe and not a broken
/// file.
#[derive(Debug, Clone)]
pub struct UnsafeVersion {
    /// Why the version is not safe to read.
    pub reason: String,
}

impl std::fmt::Display for UnsafeVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for UnsafeVersion {}

/// The reason why a version is not safe to read.
///
/// A version with a high finding and a version of the tier L3 hold
/// something that must not run, so they are not safe to read.
pub fn unsafe_reason(version: &StoredVersion) -> Option<String> {
    if version.tier == Tier::L3 {
        return Some(format!(
            "the tier of the version is {}",
            version.tier.as_str()
        ));
    }
    let high: Vec<String> = version
        .findings
        .iter()
        .filter(|finding| finding.severity == Severity::High)
        .map(|finding| catalog::redact(&finding.rule))
        .collect();
    if high.is_empty() {
        return None;
    }
    Some(format!(
        "the version has {} high {}: {}",
        high.len(),
        if high.len() == 1 {
            "finding"
        } else {
            "findings"
        },
        high.join(", ")
    ))
}

/// Refuse a version that is not safe to read.
pub fn refuse_unsafe_version(version: &StoredVersion) -> Result<()> {
    let Some(reason) = unsafe_reason(version) else {
        return Ok(());
    };
    Err(anyhow::Error::new(UnsafeVersion {
        reason: format!(
            "the stored version `{}` is not safe to read: {reason}",
            catalog::redact(&version.hash)
        ),
    }))
}

/// The record of a skill id, with a fresh one when there is none yet.
fn load_record(root: &Path, id: &str) -> Result<ThreatRecord> {
    let path = record_path(root, id);
    if !path.is_file() {
        return Ok(ThreatRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            id: catalog::redact(id),
            versions: Vec::new(),
        });
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read record `{}`", catalog::redact_path(&path)))?;
    let mut record: ThreatRecord = serde_json::from_str(&text)
        .with_context(|| format!("read record `{}`", catalog::redact_path(&path)))?;
    record.schema_version = RECORD_SCHEMA_VERSION;
    Ok(record)
}

/// Write the threat record of a skill id.
///
/// The record is written to a temporary file and moved onto its place, so a
/// record is never read half written and a record of another process is
/// never overwritten halfway.
fn write_record(root: &Path, id: &str, record: &ThreatRecord) -> Result<()> {
    let directory = root.join("records");
    create_private_dir(&directory)?;
    let mut text = serde_json::to_string_pretty(record)?;
    text.push('\n');
    let path = record_path(root, id);
    let temporary = directory.join(format!("{id}.json.tmp-{}", std::process::id()));
    fs::write(&temporary, text)
        .with_context(|| format!("write record `{}`", catalog::redact_path(&temporary)))?;
    set_file_mode(&temporary, RECORD_MODE)?;
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(anyhow::Error::new(error)
            .context(format!("write record `{}`", catalog::redact_path(&path))));
    }
    Ok(())
}

/// Create a folder of the store that only the user of the store can read.
///
/// The folder of the store, the folder of the skills, the folder of the
/// records and the folder of the locks hold the state of the store, so they
/// are private to the user of the store.
fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create `{}`", catalog::redact_path(path)))?;
    set_directory_mode(path, PRIVATE_DIR_MODE)?;
    Ok(())
}

/// The id of a skill.
///
/// The id is the one the user named with `--id`, else the identity of the
/// source: the name of the folder of a directory source or the file stem of
/// a zip archive. The identity of the source is the same whether the source
/// holds the skill directly or holds it inside a single top level folder.
///
/// The identity comes from the real path of the source, so a source that is
/// written as `.`, as `..` or as `./folder/` is named after the folder it
/// really is.
///
/// When the identity of the source is not a valid id, the name of the
/// frontmatter is used, because a skill of another author may share its
/// frontmatter name with a skill of a different author. When that name is
/// not a valid id either, the user has to name the skill with `--id`.
fn resolve_id(
    explicit: Option<&str>,
    name: Option<&str>,
    source: &Path,
    real: &Path,
) -> Result<String> {
    if let Some(id) = explicit {
        return Ok(id.trim().to_string());
    }
    let identity = source_identity(real);
    if validate_id(&identity).is_ok() {
        return Ok(identity);
    }
    if let Some(name) = name {
        let trimmed = name.trim();
        if !trimmed.is_empty() && validate_id(trimmed).is_ok() {
            return Ok(trimmed.to_string());
        }
    }
    bail!(
        "the source `{}` has no valid skill id, use `--id` to name the skill",
        catalog::redact_path(source)
    );
}

/// The identity of a source.
///
/// The identity of a directory source is the name of the directory, the
/// identity of a zip archive is the file stem of the archive.
///
/// The source is the real path of the source, so the identity of a source
/// that is written as `.` or as `..` is the name of the folder it is.
fn source_identity(source: &Path) -> String {
    let name = file_name(source);
    if source.is_dir() {
        return name;
    }
    match name.rfind('.') {
        Some(position) if position > 0 => name[..position].to_string(),
        _ => name,
    }
}

/// The root of the skill inside the staging directory.
///
/// The root holds the `SKILL.md` of the skill. A source that holds a single
/// top level folder with a `SKILL.md` inside it is rooted at that folder, so
/// a zip archive of a repository works as well as a plain folder.
///
/// A top level folder is only unwrapped when it is the only top level entry
/// of the source. A source with more than one top level entry and no
/// `SKILL.md` at its root is refused, because the skill inside it is not
/// clear.
fn skill_root(staging: &Path) -> Result<PathBuf> {
    if staging.join(SKILL_FILE).is_file() {
        return Ok(staging.to_path_buf());
    }
    let entries = read_dir_sorted(staging)?;
    if entries.len() == 1 && entries[0].is_dir() {
        let root = entries[0].clone();
        if root.join(SKILL_FILE).is_file() {
            return Ok(root);
        }
    }
    bail!("{SKILL_FILE} must be at the top level or inside a single top-level folder");
}

/// The path of the skill root, relative to the staging directory.
fn skill_prefix(root: &Path, staging: &Path) -> String {
    root.strip_prefix(staging)
        .map(|path| {
            path.to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        })
        .unwrap_or_default()
}

/// The path of a file below a prefix, when the file is inside the prefix.
fn strip_prefix(path: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(path.to_string());
    }
    let rest = path.strip_prefix(prefix)?.strip_prefix('/')?;
    Some(rest.to_string())
}

/// Stage the source into the staging directory.
///
/// A directory is copied file by file, a zip archive is extracted. The paths
/// of the symlinks of a directory are returned, because a symlink is never
/// copied and only shows up as a finding.
fn stage_source(source: &Path, staging: &Path) -> Result<Vec<String>> {
    utf8_name(source, "source")?;
    if source.is_dir() {
        return copy_directory(source, staging);
    }
    if !is_zip(source) {
        bail!(
            "source `{}` is neither a directory nor a zip archive",
            catalog::redact_path(source)
        );
    }
    extract_zip(source, staging)?;
    Ok(Vec::new())
}

/// Whether a path names a zip archive.
///
/// A file with the `.zip` extension is a zip archive, and so is a file that
/// starts with the magic number of a zip archive.
fn is_zip(path: &Path) -> bool {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        return true;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    matches!(file.read(&mut magic), Ok(4)) && &magic == b"PK\x03\x04"
}

/// Copy a directory into the staging directory.
///
/// Only regular files are copied. A symlink is never followed and never
/// copied, it only shows up as a finding. The build folders of a repository
/// are left out, they hold no part of the skill.
fn copy_directory(source: &Path, target: &Path) -> Result<Vec<String>> {
    let mut links = Vec::new();
    copy_directory_into(source, target, "", &mut links)?;
    Ok(links)
}

/// Copy the files of one directory into the staging directory.
fn copy_directory_into(
    source: &Path,
    target: &Path,
    prefix: &str,
    links: &mut Vec<String>,
) -> Result<()> {
    for path in read_dir_sorted(source)? {
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
        let name = utf8_name(&path, "entry")?;
        let relative = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if metadata.file_type().is_symlink() {
            links.push(relative);
            continue;
        }
        if metadata.is_dir() {
            if SKIPPED_FOLDERS.contains(&name.as_str()) {
                continue;
            }
            let into = target.join(&name);
            fs::create_dir_all(&into)
                .with_context(|| format!("create `{}`", catalog::redact_path(&into)))?;
            copy_directory_into(&path, &into, &relative, links)?;
        } else if metadata.is_file() {
            let into = target.join(&name);
            fs::copy(&path, &into)
                .with_context(|| format!("copy `{}`", catalog::redact_path(&path)))?;
        }
    }
    Ok(())
}

/// Extract a zip archive into the staging directory.
///
/// An archive is refused when it holds a path that leaves the archive root,
/// a symlink, more entries than the limit allows, more bytes than the limit
/// allows or an entry that is compressed far beyond its size. A refused
/// archive never reaches the store.
fn extract_zip(source: &Path, target: &Path) -> Result<()> {
    extract_zip_with_limits(
        source,
        target,
        MAX_ARCHIVE_BYTES,
        MAX_ARCHIVE_ENTRIES,
        MAX_COMPRESSION_RATIO,
    )
}

/// Extract a zip archive into the staging directory, with the given limits.
///
/// The limits are the ones of the store, but they are given as arguments so
/// a test can hold a small archive against a small limit.
fn extract_zip_with_limits(
    source: &Path,
    target: &Path,
    max_bytes: u64,
    max_entries: usize,
    max_ratio: u64,
) -> Result<()> {
    let file = fs::File::open(source)
        .with_context(|| format!("read zip archive `{}`", catalog::redact_path(source)))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("read zip archive `{}`", catalog::redact_path(source)))?;
    if archive.len() > max_entries {
        bail!(
            "zip archive holds {} entries, the limit is {max_entries}",
            archive.len()
        );
    }

    // A second handle on the archive, so the real size of the data of an
    // entry can be measured without disturbing the archive itself.
    let mut measure = fs::File::open(source)
        .with_context(|| format!("read zip archive `{}`", catalog::redact_path(source)))?;

    let mut total: u64 = 0;
    let mut written_total: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("read zip archive `{}`", catalog::redact_path(source)))?;
        let name = entry.name().to_string();
        if std::str::from_utf8(entry.name_raw()).is_err() {
            bail!(
                "zip entry `{}` has a name that is not valid UTF-8",
                catalog::redact(&String::from_utf8_lossy(entry.name_raw()))
            );
        }
        let relative = safe_relative_path(&name)?;
        if let Some(mode) = entry.unix_mode()
            && mode & 0o170000 == 0o120000
        {
            bail!("zip entry `{}` is a symlink", catalog::redact(&name));
        }
        let size = entry.size();
        let compressed = entry.compressed_size();
        if size > 0 && compressed == 0 {
            bail!(
                "zip entry `{}` has a compression ratio above {max_ratio}",
                catalog::redact(&name)
            );
        }
        if size > compressed.saturating_mul(max_ratio) {
            bail!(
                "zip entry `{}` has a compression ratio above {max_ratio}",
                catalog::redact(&name)
            );
        }
        total = total.saturating_add(size);
        if total > max_bytes {
            bail!("zip archive holds more than {max_bytes} bytes");
        }

        let path = target.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&path)
                .with_context(|| format!("create `{}`", catalog::redact_path(&path)))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create `{}`", catalog::redact_path(parent)))?;
        }
        let mut out = fs::File::create(&path)
            .with_context(|| format!("create `{}`", catalog::redact_path(&path)))?;
        // The number of compressed bytes that the entry really holds. The
        // header of the archive can lie about the size of an entry, so the
        // data of the entry is measured: the ratio of the entry is the
        // ratio of the bytes that are really there.
        let measured = match (entry.compression(), entry.data_start()) {
            (zip::CompressionMethod::Deflated, Some(start)) => {
                measure_compressed(&mut measure, start, compressed, max_bytes).with_context(
                    || format!("read zip archive `{}`", catalog::redact_path(source)),
                )?
            }
            // A stored entry is a copy, so its ratio is one.
            (zip::CompressionMethod::Stored, _) => compressed,
            _ => compressed,
        };
        // The limits are checked on the bytes that are really written, so a
        // zip that lies about the size of its entries never gets past the
        // limits. The reader of the entry is counted, and the copy stops as
        // soon as the limit is over: the bytes that are over the limit are
        // never written, the archive is refused right away and the staging
        // goes away with it.
        let remaining = max_bytes.saturating_sub(written_total);
        let entry_limit = max_ratio.saturating_mul(measured);
        let limit = remaining.min(entry_limit);
        let read = copy_entry(&mut entry, &mut out, limit)
            .with_context(|| format!("read zip archive `{}`", catalog::redact_path(source)))?;
        if read > limit {
            if entry_limit <= remaining {
                bail!(
                    "zip entry `{}` has a compression ratio above {max_ratio}",
                    catalog::redact(&name)
                );
            }
            bail!("zip archive holds more than {max_bytes} bytes");
        }
        written_total = written_total.saturating_add(read);
    }
    Ok(())
}

/// The number of compressed bytes that the data of a zip entry holds.
///
/// The data of the entry is read from the archive and decompressed, and the
/// decompressor tells how many bytes of the archive it really read. A
/// header that lies about the size of an entry cannot hide the real size
/// of the data, so the ratio of the entry is the ratio of the real bytes.
///
/// The data is read until the limit of the store is reached, so a bomb
/// never takes more time than the limit allows.
fn measure_compressed(
    file: &mut fs::File,
    start: u64,
    declared: u64,
    max_output: u64,
) -> Result<u64> {
    file.seek(SeekFrom::Start(start))
        .with_context(|| "read the data of the zip entry")?;
    let reader = BufReader::new((&mut *file).take(declared));
    let mut decoder = flate2::bufread::DeflateDecoder::new(reader);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut output = 0u64;
    loop {
        let read = decoder.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output += read as u64;
        if output > max_output {
            bail!("zip archive holds more than {max_output} bytes");
        }
    }
    Ok(decoder.total_in())
}

/// Copy the bytes of a zip entry into a file, up to a limit.
///
/// The reader of the entry is counted, so the number of bytes that are
/// really read is known even when the archive lies about the size of its
/// entry. The copy stops as soon as the limit is over: only the bytes that
/// are within the limit are written, so the file never holds more than the
/// limit.
///
/// The number of bytes that were read is returned, so the caller can tell
/// that the limit was over.
fn copy_entry(reader: &mut impl Read, out: &mut fs::File, limit: u64) -> Result<u64> {
    let mut source = reader.take(limit.saturating_add(1));
    let mut buffer = vec![0u8; 64 * 1024];
    let mut read_total = 0u64;
    let mut written = 0u64;
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        read_total += read as u64;
        if written + read as u64 > limit {
            // Only the bytes that are within the limit are written, and the
            // rest of the entry is never read.
            let room = (limit - written) as usize;
            out.write_all(&buffer[..room])?;
            break;
        }
        out.write_all(&buffer[..read])?;
        written += read as u64;
    }
    Ok(read_total)
}

/// The path of a zip entry, relative to the archive root.
///
/// A path that leaves the archive root is refused, so an archive can never
/// write outside of the staging directory.
fn safe_relative_path(name: &str) -> Result<PathBuf> {
    if name.is_empty() {
        bail!("zip entry has an empty name");
    }
    if name.starts_with('/') || name.starts_with('\\') {
        bail!("zip entry `{}` is an absolute path", catalog::redact(name));
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        bail!("zip entry `{}` is an absolute path", catalog::redact(name));
    }

    let mut parts: Vec<&str> = Vec::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => {
                if parts.pop().is_none() {
                    bail!(
                        "zip entry `{}` leaves the archive root",
                        catalog::redact(name)
                    );
                }
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        bail!("zip entry `{}` has no usable path", catalog::redact(name));
    }
    Ok(parts.iter().collect())
}

/// The content hash of a skill root.
///
/// The hash covers the path and the bytes of every stored file, so a file
/// that changed changes the hash.
///
/// Every path and every content is written with its length in front of it,
/// so two skills that hold the same bytes in another shape never hold the
/// same hash: the file `a` with the content `b` and the file `a` with the
/// content `b` of another file are two different skills.
fn content_hash(root: &Path) -> Result<(String, String, u64, u64)> {
    let mut files = Vec::new();
    collect_files(root, "", &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    let mut bytes: u64 = 0;
    for relative in &files {
        let content = fs::read(root.join(relative))
            .with_context(|| format!("read `{}`", catalog::redact(relative)))?;
        bytes += content.len() as u64;
        hasher.update((relative.len() as u64).to_be_bytes());
        hasher.update(relative.as_bytes());
        hasher.update((content.len() as u64).to_be_bytes());
        hasher.update(&content);
    }
    let full = hex(&hasher.finalize());
    let hash = full[..HASH_LENGTH].to_string();
    Ok((hash, full, files.len() as u64, bytes))
}

/// The name of the folder of a version of a skill.
///
/// The name is the short hash of the version, so the versions of a skill sit
/// in folders of their own. When a folder of that name is already there and
/// holds another version, a longer part of the hash names the version, so
/// two versions of a skill never share a folder and one version is never
/// taken for another one.
fn version_name(root: &Path, id: &str, full_hash: &str) -> Result<String> {
    let parent = root.join("skills").join(id);
    for length in [HASH_LENGTH, HASH_LENGTH * 2, full_hash.len()] {
        if length > full_hash.len() {
            break;
        }
        let name = full_hash[..length].to_string();
        let target = parent.join(&name);
        if !target.exists() {
            return Ok(name);
        }
        if !target.is_dir() {
            continue;
        }
        // The folder is already there: it holds this version when the copy
        // of the folder has the same hash.
        if content_hash(&target)
            .map(|stored| stored.1 == full_hash)
            .unwrap_or(false)
        {
            return Ok(name);
        }
    }
    bail!(
        "the version `{}` of the skill `{}` is already stored",
        catalog::redact(&full_hash[..HASH_LENGTH]),
        catalog::redact(id)
    )
}

/// What the scan of a skill root found.
struct Scan {
    /// The scripts of the skill.
    scripts: Vec<ScannedScript>,
    /// The findings of the threat scan.
    findings: Vec<Finding>,
    /// Whether a python script of the skill opens a file.
    opens_file: bool,
}

/// One script of a skill, with the modules of the skill itself.
struct ScannedScript {
    /// The script itself.
    script: StoredScript,
    /// The path of the script, relative to the skill root, as it is.
    ///
    /// The path of the script itself is redacted, because a path can hold a
    /// secret. The scan needs the real path to know the folder of the
    /// script, so the real path is kept beside the redacted one.
    path: String,
    /// The modules that the script can import from the skill itself.
    local_modules: BTreeSet<String>,
    /// The language that the script really runs as.
    ///
    /// A script whose shebang and whose extension disagree runs as the
    /// interpreter of its shebang, so the more restrictive of the two
    /// languages is the one that counts for the tier.
    effective_language: &'static str,
}

/// The scripts and the findings of a skill root.
///
/// The scripts are found the same way the catalog finds them, so a skill
/// holds the same scripts in the store as in the index. The findings hold the
/// place of a secret, never its value.
fn scan_skill(root: &Path) -> Result<Scan> {
    let mut files = Vec::new();
    collect_files(root, "", &mut files)?;
    files.sort();

    let modules = folder_modules(&files);
    let mut scripts = Vec::new();
    let mut findings = Vec::new();
    let mut opens_file = false;
    for relative in &files {
        let content = fs::read(root.join(relative))
            .with_context(|| format!("read `{}`", catalog::redact(relative)))?;
        let binary = is_binary(relative, &content);
        let text = String::from_utf8_lossy(&content);
        let extension_language = catalog::script_language(Path::new(relative));
        let interpreter = catalog::shebang_language(&text);
        let language = script_language(relative, &text);
        // A file whose shebang and whose extension disagree is run by the
        // interpreter of its shebang, so the more restrictive of the two
        // languages is the one that counts.
        let mut effective_language = language;
        if let (Some(extension), Some(interpreter)) = (extension_language, interpreter)
            && extension != interpreter
        {
            effective_language = Some(stricter_language(extension, interpreter));
            findings.push(Finding {
                rule: "mismatched_shebang".to_string(),
                severity: Severity::Medium,
                path: catalog::redact(relative),
                line: Some(1),
                detail: None,
            });
        }
        if let Some(language) = language {
            let imports = if language == "python" {
                catalog::python_imports(&text)
                    .iter()
                    .map(|item| catalog::redact(item))
                    .collect()
            } else {
                Vec::new()
            };
            if language == "python" && risk::calls_open(&text) {
                opens_file = true;
            }
            scripts.push(ScannedScript {
                script: StoredScript {
                    path: catalog::redact(relative),
                    language: language.to_string(),
                    imports,
                },
                path: relative.clone(),
                local_modules: local_modules(&modules, relative),
                effective_language: effective_language.unwrap_or(language),
            });
        }
        if binary {
            findings.push(Finding {
                rule: "binary".to_string(),
                severity: Severity::High,
                path: catalog::redact(relative),
                line: None,
                detail: None,
            });
        }
        // Every file is scanned, also a file that holds bytes that are not
        // text. The bytes are cut into lines and every line is read as text,
        // so a secret inside a binary payload is found as well.
        // A type of Python holds no secret, but only in a file of python.
        let python = catalog::is_python_file(Path::new(relative), &text);
        for (index, line) in content.split(|byte| *byte == b'\n').enumerate() {
            let line = String::from_utf8_lossy(line);
            if catalog::line_has_secret(&line, python) {
                findings.push(Finding {
                    rule: "secret".to_string(),
                    severity: Severity::Medium,
                    path: catalog::redact(relative),
                    line: Some(index as u64 + 1),
                    detail: None,
                });
            }
        }
        // A python file that cannot be tokenised hides its imports, so the
        // scan cannot say what the file does. The file is a finding of its
        // own, because a file that cannot be read is a file that can hold
        // anything.
        if language == Some("python") && !catalog::python_parses(&text) {
            findings.push(Finding {
                rule: "unparsed".to_string(),
                severity: Severity::Medium,
                path: catalog::redact(relative),
                line: None,
                detail: None,
            });
        }
        // The risk rules read the text of a file. A file that holds a binary
        // payload and a file that is too big are left to the findings above.
        // A file that is too big is not read at all, so it is a finding of
        // its own: a file that is not scanned can hold anything.
        if content.len() as u64 > risk::MAX_SCAN_BYTES {
            findings.push(Finding {
                rule: "unscanned".to_string(),
                severity: Severity::Medium,
                path: catalog::redact(relative),
                line: None,
                detail: None,
            });
        } else if !binary {
            findings.extend(risk::scan_text(&catalog::redact(relative), &text));
        }
    }
    // A file of the skill that hides a module of the standard library is a
    // finding of its own, because the interpreter runs the file of the
    // skill where it would run the module of the standard library. The
    // finding keeps the name of the module that the file hides.
    for (path, module) in shadowed_modules(&files, &scripts) {
        findings.push(Finding {
            rule: "shadows_stdlib".to_string(),
            severity: Severity::Medium,
            path: catalog::redact(&path),
            line: None,
            detail: Some(module.to_string()),
        });
    }
    Ok(Scan {
        scripts,
        findings,
        opens_file,
    })
}

/// The python modules of every folder of the skill.
///
/// The map holds, for every folder of the skill, the names of the modules
/// that a script of that folder can import from the skill itself: the name
/// of a python file of the folder, and the name of a folder of the folder
/// that holds a python file of its own. A folder that holds only other
/// files holds no module, so a skill that reads a folder of notes does not
/// look like a skill that needs a package from the network.
fn folder_modules(files: &[String]) -> BTreeMap<String, BTreeSet<String>> {
    let mut modules: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut packages: BTreeSet<String> = BTreeSet::new();
    for file in files {
        let Some(stem) = name_of(file).strip_suffix(".py") else {
            continue;
        };
        let folder = folder_of(file);
        // A folder that holds a python file is a package of the skill.
        packages.insert(folder.to_string());
        if !stem.is_empty() && stem != "__init__" {
            modules
                .entry(folder.to_string())
                .or_default()
                .insert(stem.to_string());
        }
    }
    for package in &packages {
        if package.is_empty() {
            continue;
        }
        let name = name_of(package);
        if name.is_empty() {
            continue;
        }
        modules
            .entry(folder_of(package).to_string())
            .or_default()
            .insert(name.to_string());
    }
    modules
}

/// The modules that a script of the skill can import from the skill itself.
///
/// A module is local when a folder of the script holds a python file of that
/// name, or a folder of that name that holds a python file of its own.
/// Every folder above the script counts as well, up to the root of the
/// skill, because the interpreter puts the folder of the entry script on the
/// path of the modules: a script of `scripts/migrate` can import the package
/// `utils` of `scripts`.
///
/// A python file in a folder beside the script is no module of the script,
/// so `other/data.py` does not make `data` a module of a script of the root
/// of the skill, and `data/sub/helper.py` does not make `data` a module
/// either, because the folder `data` holds no python file of its own.
///
/// A module that reaches outside of the process is never local, so a skill
/// that holds a file of the name of such a module still needs a container.
fn local_modules(modules: &BTreeMap<String, BTreeSet<String>>, script: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut folder = folder_of(script);
    loop {
        if let Some(found) = modules.get(folder) {
            names.extend(found.iter().cloned());
        }
        if folder.is_empty() {
            break;
        }
        folder = folder_of(folder);
    }
    names.retain(|name| !risk::RISKY_MODULES.contains(&name.as_str()));
    names
}

/// The files and the folders of the skill that hide a module of the
/// standard library.
///
/// A file of the name of a module of the standard library hides that module,
/// and a folder of that name that holds a python file below it hides it as
/// well. The interpreter runs the file of the skill where it would run the
/// module of the standard library, so such a file is a finding of its own.
///
/// Only a file or a folder that sits in a folder where a script lives, or
/// in a folder above such a folder, hides a module: a file of a folder that
/// holds no script and that is below no script is never imported, so it
/// hides nothing.
///
/// Every shadow holds the path of the file or of the folder and the name of
/// the module that it hides, so the finding of the shadow says what the
/// file of the skill stands in the way of.
fn shadowed_modules(files: &[String], scripts: &[ScannedScript]) -> Vec<(String, String)> {
    // The folders that hold a script and the folders above them.
    let mut script_folders: BTreeSet<&str> = BTreeSet::new();
    for script in scripts {
        let mut folder = folder_of(&script.path);
        loop {
            script_folders.insert(folder);
            if folder.is_empty() {
                break;
            }
            folder = folder_of(folder);
        }
    }
    // The folders that hold a python file below them.
    let mut packages: BTreeSet<&str> = BTreeSet::new();
    for file in files {
        if !file.ends_with(".py") {
            continue;
        }
        let mut folder = folder_of(file);
        packages.insert(folder);
        while !folder.is_empty() {
            folder = folder_of(folder);
            packages.insert(folder);
        }
    }
    let mut shadows: BTreeSet<(String, String)> = BTreeSet::new();
    for file in files {
        if !script_folders.contains(folder_of(file)) {
            continue;
        }
        if let Some(module) = risk::hidden_stdlib_module(name_of(file)) {
            shadows.insert((file.clone(), module.to_string()));
        }
    }
    for file in files {
        let mut folder = folder_of(file);
        while !folder.is_empty() {
            if packages.contains(folder)
                && script_folders.contains(folder_of(folder))
                && let Some(module) = risk::hidden_stdlib_module(name_of(folder))
            {
                shadows.insert((folder.to_string(), module.to_string()));
            }
            folder = folder_of(folder);
        }
    }
    shadows.into_iter().collect()
}

/// The language of a script of the skill.
///
/// A file with a known extension is a script of that language. A file
/// without a known extension is a script when its first line starts with a
/// `#!`, and the interpreter of that line names the language of the script.
///
/// An interpreter that is not known is the language `other`, and a script of
/// an unknown language needs a container.
fn script_language(relative: &str, text: &str) -> Option<&'static str> {
    if let Some(language) = catalog::script_language(Path::new(relative)) {
        return Some(language);
    }
    catalog::shebang_language(text)
}

/// The language that a script really runs as.
///
/// A script whose shebang and whose extension disagree is run by the
/// interpreter of its shebang, so the more restrictive of the two languages
/// is the one that counts. A python script is the only script that can
/// stay in a plain process, so a script that is not python needs a
/// container.
fn stricter_language(extension: &'static str, interpreter: &'static str) -> &'static str {
    if extension == "python" {
        interpreter
    } else {
        extension
    }
}

/// The folder of a path of the skill.
///
/// The folder of a file of the root of the skill is empty.
fn folder_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(position) => &path[..position],
        None => "",
    }
}

/// The name of a path of the skill.
fn name_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(position) => &path[position + 1..],
        None => path,
    }
}

/// The tier that the scan suggests for a skill.
///
/// The tier says how much room a skill needs. A high finding means the
/// skill never runs outside a microVM. A script that was not read at all
/// means the same, because such a script can hide anything. A script that is
/// not python, a module that is not in the standard library, a module that
/// reaches outside of the process, a file that was not read, a file that
/// cannot be parsed and a file that hides a module of the standard library
/// mean a container. A module that touches the file system and a script that
/// opens a file mean a sandbox. A script that only computes means a plain
/// process. A skill without a script is guidance and nothing to run.
fn suggest_tier(
    scripts: &[ScannedScript],
    opens_file: bool,
    findings: &[Finding],
) -> (Tier, Vec<String>) {
    let mut reasons: BTreeSet<String> = BTreeSet::new();
    for finding in findings {
        if finding.severity == Severity::High {
            reasons.insert(finding.rule.clone());
        }
    }
    if !reasons.is_empty() {
        return (Tier::L3, reasons.into_iter().collect());
    }

    // A script that was not read at all can hide anything, so it needs a
    // microVM and not only a container.
    for finding in findings {
        if finding.rule != "unscanned" {
            continue;
        }
        if scripts
            .iter()
            .any(|scanned| scanned.script.path == finding.path)
        {
            reasons.insert(format!("unscanned script: {}", finding.path));
        }
    }
    if !reasons.is_empty() {
        return (Tier::L3, reasons.into_iter().collect());
    }

    for scanned in scripts {
        if scanned.effective_language == "python" {
            continue;
        }
        if scanned.effective_language == scanned.script.language {
            reasons.insert(format!(
                "{} script {}",
                scanned.script.language, scanned.script.path
            ));
        } else {
            reasons.insert(format!(
                "{} script {} (the shebang does not match the extension)",
                scanned.effective_language, scanned.script.path
            ));
        }
    }
    for scanned in scripts {
        if scanned.effective_language != "python" {
            continue;
        }
        for import in &scanned.script.imports {
            // A module that reaches outside of the process is risky, even
            // when the skill holds a file of that name, so the skill needs
            // a container and not only a sandbox.
            if risk::RISKY_MODULES.contains(&import.as_str()) {
                reasons.insert(format!("risky import: {import}"));
            } else if !risk::STDLIB_MODULES.contains(&import.as_str())
                && !scanned.local_modules.contains(import)
            {
                reasons.insert(format!("third party import: {import}"));
            }
        }
    }
    // A file that hides a module that reaches outside of the process needs
    // a microVM: the interpreter runs the file of the skill where it would
    // run the module of the interpreter, so the skill can do everything
    // that the module can do.
    let mut shadows: BTreeSet<String> = BTreeSet::new();
    for finding in findings {
        if finding.rule != "shadows_stdlib" {
            continue;
        }
        let Some(module) = finding.detail.as_deref() else {
            continue;
        };
        if risk::RISKY_MODULES.contains(&module) {
            shadows.insert(format!(
                "shadows the module {module} that reaches outside of the process: {}",
                finding.path
            ));
        }
    }
    if !shadows.is_empty() {
        reasons.extend(shadows);
        return (Tier::L3, reasons.into_iter().collect());
    }

    // A file that was not read and a file that cannot be parsed need a
    // container: the scan cannot say what the file does. A file that hides
    // a module of the standard library needs a container as well, because
    // the interpreter runs the file of the skill where it would run the
    // module of the interpreter. A skill that reaches outside of the
    // process is already a container or more, so a file that hides such a
    // module never needs more than a container.
    for finding in findings {
        let reason = match finding.rule.as_str() {
            "unscanned" => Some(format!("unscanned file: {}", finding.path)),
            "unparsed" => Some(format!("unparsed file: {}", finding.path)),
            "shadows_stdlib" => Some(format!(
                "shadows the module {} of the standard library: {}",
                finding.detail.as_deref().unwrap_or("a module"),
                finding.path
            )),
            // An import whose name is only known at the time of the call
            // can name any module, so the skill needs a container.
            "dynamic_import" => Some("dynamic_import".to_string()),
            _ => None,
        };
        if let Some(reason) = reason {
            reasons.insert(reason);
        }
    }
    if !reasons.is_empty() {
        return (Tier::L2, reasons.into_iter().collect());
    }

    for scanned in scripts {
        if scanned.effective_language != "python" {
            continue;
        }
        for import in &scanned.script.imports {
            if risk::FILESYSTEM_MODULES.contains(&import.as_str()) {
                reasons.insert(format!("file system import: {import}"));
            }
        }
    }
    if opens_file {
        reasons.insert("opens a file".to_string());
    }
    if !reasons.is_empty() {
        return (Tier::L1, reasons.into_iter().collect());
    }

    if scripts
        .iter()
        .any(|scanned| scanned.effective_language == "python")
    {
        return (Tier::L0, vec!["python scripts only".to_string()]);
    }
    (Tier::None, vec!["no scripts".to_string()])
}

/// Whether a file holds a binary payload.
///
/// A file is a binary when it starts with the magic number of a binary
/// format or when its extension names one.
fn is_binary(relative: &str, content: &[u8]) -> bool {
    if let Some(extension) = Path::new(relative)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        let extension = extension.to_ascii_lowercase();
        if BINARY_EXTENSIONS.contains(&extension.as_str()) {
            return true;
        }
    }
    BINARY_MAGICS.iter().any(|magic| content.starts_with(magic))
}

/// The paths of the regular files below a root, relative to that root.
fn collect_files(root: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
    for path in read_dir_sorted(root)? {
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let name = file_name(&path);
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if metadata.is_dir() {
            collect_files(&path, &relative, out)?;
        } else if metadata.is_file() {
            out.push(relative);
        }
    }
    Ok(())
}

/// Copy a skill root into the store, with read-only files.
fn copy_readonly(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)
        .with_context(|| format!("create `{}`", catalog::redact_path(target)))?;
    set_directory_mode(target, PRIVATE_DIR_MODE)?;
    for path in read_dir_sorted(source)? {
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
        let name = file_name(&path);
        let into = target.join(&name);
        if metadata.is_dir() {
            copy_readonly(&path, &into)?;
        } else if metadata.is_file() {
            fs::copy(&path, &into)
                .with_context(|| format!("copy `{}`", catalog::redact_path(&path)))?;
            set_file_mode(&into, STORED_FILE_MODE)?;
        }
    }
    Ok(())
}

/// Make every directory of a stored skill read-only.
fn lock_directories(root: &Path) -> Result<()> {
    let mut directories = vec![root.to_path_buf()];
    collect_directories(root, &mut directories)?;
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        set_directory_mode(&directory, STORED_DIR_MODE)?;
    }
    Ok(())
}

/// The directories below a root.
fn collect_directories(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for path in read_dir_sorted(root)? {
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read `{}`", catalog::redact_path(&path)))?;
        if metadata.is_dir() {
            out.push(path.clone());
            collect_directories(&path, out)?;
        }
    }
    Ok(())
}

/// Set the mode of a file.
#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("set the mode of `{}`", catalog::redact_path(path)))
}

/// Set the mode of a file.
#[cfg(not(unix))]
fn set_file_mode(path: &Path, mode: u32) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("read `{}`", catalog::redact_path(path)))?
        .permissions();
    permissions.set_readonly(mode & 0o222 == 0);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("set the mode of `{}`", catalog::redact_path(path)))
}

/// Set the mode of a directory.
#[cfg(unix)]
fn set_directory_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("set the mode of `{}`", catalog::redact_path(path)))
}

/// Set the mode of a directory.
#[cfg(not(unix))]
fn set_directory_mode(path: &Path, mode: u32) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("read `{}`", catalog::redact_path(path)))?
        .permissions();
    permissions.set_readonly(mode & 0o222 == 0);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("set the mode of `{}`", catalog::redact_path(path)))
}

/// The entries of a directory, sorted by path.
fn read_dir_sorted(directory: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(directory)
        .with_context(|| format!("read `{}`", catalog::redact_path(directory)))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("read `{}`", catalog::redact_path(directory)))?;
        paths.push(entry.path());
    }
    paths.sort();
    Ok(paths)
}

/// The name of a path, as text.
///
/// A name that is not valid UTF-8 is refused, because such a name cannot be
/// written into a record and cannot be printed. The name is shown lossily
/// and goes through the redaction helper, so a secret shaped name never
/// reaches the output.
fn utf8_name(path: &Path, kind: &str) -> Result<String> {
    let Some(raw) = path.file_name() else {
        return Ok(String::new());
    };
    let Some(name) = raw.to_str() else {
        bail!(
            "the {kind} `{}` has a name that is not valid UTF-8",
            catalog::redact(&raw.to_string_lossy())
        );
    };
    Ok(name.to_string())
}

/// The name of a path, as text.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The hex text of some bytes.
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        text.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    text
}

/// The current time in nanoseconds.
fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

/// A time as RFC 3339 in UTC.
fn rfc3339(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// The year, the month and the day of a number of days since the epoch.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_of_year = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_of_year + 2) / 5 + 1) as u32;
    let month = if month_of_year < 10 {
        month_of_year + 3
    } else {
        month_of_year - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// The lock of one skill id.
///
/// The lock is a file that only one process can create, so two adds of the
/// same skill id never write the same record at the same time.
///
/// The process that took the lock holds the lock of the file as well, so a
/// process that wants to reclaim a stale lock has to wait for the process
/// that holds it. The file holds the pid of the process that took it, so a
/// lock of a process that is gone can be told from a lock of a process that
/// runs. A lock that is older than ten minutes is reclaimed all the same,
/// because the add that took it never finished.
///
/// The file goes away with the lock, also when the add fails. The file is
/// only removed when it is still the file of this process, so the release of
/// a lock never removes the lock of another process.
#[derive(Debug)]
struct Lock {
    path: PathBuf,
    /// The open file that holds the lock of the file.
    ///
    /// The handle stays open as long as the lock is held, so the lock of
    /// the file is held as long as the add works.
    file: Option<fs::File>,
}

impl Lock {
    /// Take the lock of a skill id, waiting for it to go away.
    fn acquire(root: &Path, id: &str) -> Result<Self> {
        Self::acquire_within(root, id, LOCK_TIMEOUT)
    }

    /// Take the lock of a skill id, waiting at most for the given time.
    ///
    /// The wait is short, so a lock of a process that died on its own goes
    /// away soon enough.
    fn acquire_within(root: &Path, id: &str, timeout: Duration) -> Result<Self> {
        create_private_dir(root)?;
        let directory = root.join("locks");
        create_private_dir(&directory)?;
        let path = directory.join(format!("{id}.lock"));
        let deadline = Instant::now() + timeout;
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let _ = set_file_mode(&path, RECORD_MODE);
                    // The lock of the file keeps a process that reclaims a
                    // stale lock away, so the lock of this process stays
                    // while the add works. The wait is short, because only
                    // a process that reads the lock of the file can hold
                    // it.
                    lock_file(&file);
                    // The lock holds the pid of the process that took it,
                    // so a lock of a process that is gone can be told from
                    // a lock of a process that runs.
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if remove_stale_lock(&path) {
                        // The lock was stale, so the add goes on without
                        // waiting for it.
                        continue;
                    }
                    if Instant::now() >= deadline {
                        bail!(
                            "the skill id `{}` is locked by another add, the lock `{}` was held for {} seconds",
                            catalog::redact(id),
                            catalog::redact_path(&path),
                            timeout.as_secs()
                        );
                    }
                    std::thread::sleep(LOCK_RETRY);
                }
                Err(error) => {
                    return Err(anyhow::Error::new(error)
                        .context(format!("take the lock `{}`", catalog::redact_path(&path))));
                }
            }
        }
    }
}

/// Whether a lock is stale and can be removed.
///
/// A lock of a process that is gone is stale, and a lock that is older than
/// ten minutes is stale as well, because the add that took it never
/// finished.
///
/// The lock of the file is taken before the lock is read, so a young lock
/// that a process holds is never removed: the process works on the skill,
/// so its lock is no lock of a process that is gone. A lock that is older
/// than ten minutes is removed all the same, because a lock that is that
/// old is a lock of an add that never finished.
///
/// The file that is removed is the file that was read, so a lock that
/// another add took in the meantime stays.
fn remove_stale_lock(path: &Path) -> bool {
    let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(path) else {
        return false;
    };
    // The lock of the file is taken, so a process that reclaims a lock at
    // the same time waits for this one.
    let held = take_lock(&file);
    // The age of the lock is read before the lock is judged, so a lock
    // that is old is removed even when a process holds the lock of the
    // file.
    let old = older_than(path, LOCK_STALE_AFTER);
    if !old && !held {
        // A process holds the lock of the file and the lock is young, so
        // the process works on the skill and its lock is no stale lock.
        return false;
    }
    let Some(content) = read_lock(&file) else {
        return false;
    };
    if !same_file(&file, path) {
        // The path names another file now, so the lock that was read is no
        // lock of the path.
        return false;
    }
    if !stale_lock(path, &content) {
        return false;
    }
    fs::remove_file(path).is_ok()
}

/// Whether a file is older than the given time.
///
/// The age of a file is the time since it was written the last time, so a
/// lock that is older than ten minutes is a lock of an add that never
/// finished.
fn older_than(path: &Path, age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|elapsed| elapsed > age)
}

/// Whether the lock of the file was taken without waiting.
///
/// The lock of the file is the lock of the system, so a process that holds
/// it keeps every other process away from the file. The lock is taken
/// without waiting, so a process never waits for another one here.
///
/// A process that holds the lock of the file works on the skill, so its
/// lock is no stale lock.
///
/// Without a system of unix there is no such lock, so the file is never
/// locked by another process.
#[cfg(unix)]
fn take_lock(file: &fs::File) -> bool {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    // The lock is exclusive and it is taken without waiting.
    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;
    unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) == 0 }
}

/// Whether the lock of the file was taken without waiting.
///
/// Without a system of unix there is no such lock, so the file is never
/// locked by another process.
#[cfg(not(unix))]
fn take_lock(_file: &fs::File) -> bool {
    true
}

/// Whether the lock of the file was taken, waiting for the process that
/// holds it.
///
/// The wait is short, because the process that holds the lock of the file
/// only reads the file and goes away.
#[cfg(unix)]
fn lock_file(file: &fs::File) -> bool {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    // The lock is exclusive and it waits for the process that holds it.
    const LOCK_EX: i32 = 2;
    unsafe { flock(file.as_raw_fd(), LOCK_EX) == 0 }
}

/// Whether the lock of the file was taken, waiting for the process that
/// holds it.
///
/// Without a system of unix there is no such lock, so the file is never
/// locked by another process.
#[cfg(not(unix))]
fn lock_file(_file: &fs::File) -> bool {
    true
}

/// Whether the path names the file that was opened.
///
/// The path can name another file when another process took the lock in
/// the meantime, so the file that is removed is the file that was read.
#[cfg(unix)]
fn same_file(file: &fs::File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(opened) = file.metadata() else {
        return false;
    };
    let Ok(current) = fs::metadata(path) else {
        return false;
    };
    opened.dev() == current.dev() && opened.ino() == current.ino()
}

/// Whether the path names the file that was opened.
///
/// Without a system of unix the files are told apart by their text, so a
/// path that holds another text names another file.
#[cfg(not(unix))]
fn same_file(file: &fs::File, path: &Path) -> bool {
    let Some(opened) = read_lock(file) else {
        return false;
    };
    let Ok(current) = fs::read_to_string(path) else {
        return false;
    };
    opened == current
}

/// The text of a lock file.
///
/// The text is read from the open file, so the text is the text of the file
/// that was opened and not the text of another file that took its place.
fn read_lock(file: &fs::File) -> Option<String> {
    use std::io::Read;
    let mut content = String::new();
    let mut handle = file;
    handle.read_to_string(&mut content).ok()?;
    Some(content)
}

/// Whether a lock is stale.
///
/// A lock holds the pid of the process that took it. A lock of a process
/// that is gone is stale, and a lock that is older than ten minutes is
/// stale as well.
///
/// A young lock that a process holds is no stale lock: the process works
/// on the skill, so the lock is no lock of a process that is gone. The
/// lock of the file is taken before the lock is read, so only a lock that
/// no process holds is read here. A lock that is older than ten minutes is
/// stale all the same, because the add that took it never finished.
fn stale_lock(path: &Path, content: &str) -> bool {
    if older_than(path, LOCK_STALE_AFTER) {
        return true;
    }
    let Some(pid) = content
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<i32>().ok())
    else {
        // A lock that holds no pid is no lock of a known process, so it is
        // only stale when it is old.
        return false;
    };
    !process_is_alive(pid)
}

/// Whether a process with the given pid is alive.
///
/// The check is the `kill` of the system with the signal `0`, so the
/// process is never touched. The check fails with `ESRCH` when no process
/// holds the pid.
#[cfg(unix)]
fn process_is_alive(pid: i32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // The signal `0` asks for the state of the process and touches
    // nothing.
    if unsafe { kill(pid, 0) } == 0 {
        return true;
    }
    // The check fails with `ESRCH` when no process holds the pid. Every
    // other failure means that the process is there but that the check may
    // not ask it anything.
    std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
}

/// The number of the error of the system that says that there is no such
/// process.
///
/// The number is the same on every system of unix.
#[cfg(unix)]
const ESRCH: i32 = 3;

/// Whether a process with the given pid is alive.
///
/// Without a system of unix there is no way to ask, so every process is
/// assumed to be alive.
#[cfg(not(unix))]
fn process_is_alive(_pid: i32) -> bool {
    true
}

impl Drop for Lock {
    fn drop(&mut self) {
        // The lock of the file is still held, so no other process can
        // remove the file, and the file is only removed when it is still
        // the file of this process, so the release of a lock never removes
        // the lock of another process.
        if let Some(file) = &self.file
            && same_file(file, &self.path)
        {
            let _ = fs::remove_file(&self.path);
        }
        // The lock of the file goes away with the handle of the file.
    }
}

/// A staging directory that goes away with the command.
struct Staging {
    path: PathBuf,
}

impl Staging {
    /// A fresh staging directory below the temporary directory.
    fn new() -> Result<Self> {
        Self::named("skrun-store")
    }

    /// A fresh staging directory below the temporary directory, with a name
    /// of its own.
    fn named(prefix: &str) -> Result<Self> {
        for attempt in 0..64u32 {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("{prefix}-{}-{stamp}-{attempt}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    set_directory_mode(&path, PRIVATE_DIR_MODE)?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(anyhow::Error::new(error).context(format!(
                        "create staging directory `{}`",
                        catalog::redact_path(&path)
                    )));
                }
            }
        }
        bail!("no staging directory could be created")
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = unlock_directories(&self.path);
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Make every directory of a tree writable again.
fn unlock_directories(root: &Path) -> Result<()> {
    let mut directories = vec![root.to_path_buf()];
    collect_directories(root, &mut directories)?;
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        set_directory_mode(&directory, PRIVATE_DIR_MODE)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    /// A ref with an underscore, such as the name of a branch, is accepted;
    /// a ref with `..` or a leading dash is still refused.
    #[test]
    fn github_ref_accepts_an_underscore() {
        assert!(check_github_ref("feature_branch").is_ok());
        assert!(check_github_ref("release/v1_2").is_ok());
        assert!(check_github_ref("a..b").is_err());
        assert!(check_github_ref("-x_y").is_err());
    }

    /// A key shaped value that must never reach the output.
    const SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

    /// The mode of a file or of a directory.
    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
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

    /// A skill directory with a python script and a planted key.
    ///
    /// The skill lives in a folder named `demo`, because the id of a skill
    /// comes from the name of its source.
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

    /// Make a folder of the store writable again.
    ///
    /// The copy in the store is read only, so a test that puts a file into
    /// it or that removes a file of it has to open the folder first.
    fn make_writable(path: &Path) {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).unwrap();
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap().path();
            if entry.is_dir() && !entry.is_symlink() {
                make_writable(&entry);
            } else {
                let mut permissions = fs::metadata(&entry).unwrap().permissions();
                permissions.set_mode(0o600);
                fs::set_permissions(&entry, permissions).unwrap();
            }
        }
    }

    /// The options of an add of a source into a store.
    fn options(source: &Path, store: &Path) -> AddOptions {
        AddOptions {
            source: source.to_path_buf(),
            store: Some(store.to_path_buf()),
            id: None,
            from: None,
        }
    }

    /// A zip archive with one entry whose name holds bytes that are not
    /// UTF-8.
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

    /// A zip archive with the given entries.
    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let file = fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let entry_options =
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, content) in entries {
            writer.start_file(*name, entry_options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn a_module_of_a_folder_above_the_script_is_a_local_module() {
        let files = vec![
            "scripts/migrate/agents.py".to_string(),
            "scripts/utils/__init__.py".to_string(),
            "scripts/utils.py".to_string(),
            "other/data.py".to_string(),
        ];
        let modules = folder_modules(&files);
        let local = local_modules(&modules, "scripts/migrate/agents.py");

        assert!(local.contains("utils"), "{local:?}");
        assert!(!local.contains("data"), "{local:?}");
    }

    #[test]
    fn a_risky_module_is_never_a_local_module() {
        let files = vec![
            "scripts/migrate/agents.py".to_string(),
            "scripts/socket.py".to_string(),
        ];
        let modules = folder_modules(&files);
        let local = local_modules(&modules, "scripts/migrate/agents.py");

        assert!(!local.contains("socket"), "{local:?}");
    }

    #[test]
    fn adds_a_directory_skill_with_a_planted_secret() {
        let source = skill_fixture("add-directory");
        let store = temp_dir("add-directory-store");

        let result = add_skill(&options(&source, &store)).unwrap();

        assert_eq!(result.id, "demo");
        assert!(!result.already_stored);
        assert_eq!(result.version.files, 2);
        assert_eq!(result.version.hash.len(), HASH_LENGTH);
        assert_eq!(result.version.full_hash.len(), 64);
        assert_eq!(result.version.name.as_deref(), Some("demo"));

        let secret = result
            .version
            .findings
            .iter()
            .find(|finding| finding.rule == "secret")
            .expect("a secret finding");
        assert_eq!(secret.path, "run.py");
        assert_eq!(secret.line, Some(3));
        assert_eq!(secret.severity, Severity::Medium);
        assert_eq!(result.version.tier, "L1");

        let script = result
            .version
            .scripts
            .iter()
            .find(|script| script.path == "run.py")
            .expect("a script");
        assert_eq!(script.language, "python");
        assert_eq!(script.imports, vec!["os".to_string()]);

        let record = fs::read_to_string(store.join("records/demo.json")).unwrap();
        assert!(record.contains("\"secret\""), "{record}");
        assert!(!record.contains(SECRET), "{record}");
    }

    #[test]
    fn the_hash_writes_the_length_of_every_path_and_of_every_content() {
        let root = temp_dir("hash-length");
        write(&root.join("a"), "bc");
        let (hash, full, files, bytes) = content_hash(&root).unwrap();

        // The hash of the file `a` with the content `bc` is the length of
        // the path, the path, the length of the content and the content.
        assert_eq!(
            full,
            "3fafa1cf2f19a7c1129beb20cf0983f73a489a221fc0dd2f16d1be292d089205"
        );
        assert_eq!(hash, full[..HASH_LENGTH]);
        assert_eq!(files, 1);
        assert_eq!(bytes, 2);
    }

    #[test]
    fn two_shapes_of_the_same_bytes_hold_two_hashes() {
        let first = temp_dir("hash-shape-first");
        write(&first.join("a"), "\u{0}b\u{0}c");
        let second = temp_dir("hash-shape-second");
        write(&second.join("a"), "");
        write(&second.join("b"), "c");

        let one = content_hash(&first).unwrap().1;
        let two = content_hash(&second).unwrap().1;

        // The old hash joined the path and the content with a null byte, so
        // the two shapes held the same hash.
        assert_ne!(one, two);
        assert_eq!(
            two,
            "67828f4f00b886c92b0a49b80f76c7317caa87d8501a0d4cb2a35be3fa209cfd"
        );
    }

    #[test]
    fn a_folder_of_the_short_hash_that_holds_another_version_gets_a_longer_name() {
        let store = temp_dir("version-name-store");
        let parent = store.join("skills").join("demo");
        fs::create_dir_all(&parent).unwrap();
        let full = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        fs::create_dir_all(parent.join(&full[..HASH_LENGTH])).unwrap();
        write(
            &parent.join(&full[..HASH_LENGTH]).join("other.txt"),
            "other\n",
        );

        let name = version_name(&store, "demo", full).unwrap();

        assert_eq!(name, full[..HASH_LENGTH * 2]);
    }

    #[test]
    fn a_folder_of_the_short_hash_that_holds_the_version_keeps_its_name() {
        let store = temp_dir("version-name-same-store");
        let parent = store.join("skills").join("demo");
        fs::create_dir_all(&parent).unwrap();
        let full = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let folder = parent.join(&full[..HASH_LENGTH]);
        fs::create_dir_all(&folder).unwrap();
        write(&folder.join("a"), "bc");
        let (_, stored, _, _) = content_hash(&folder).unwrap();

        let name = version_name(&store, "demo", &stored).unwrap();

        assert_eq!(name, stored[..HASH_LENGTH]);
    }

    #[test]
    fn the_copy_of_a_zip_entry_stops_at_the_limit() {
        let path = temp_dir("copy-entry").join("out.bin");
        let mut out = fs::File::create(&path).unwrap();
        let content: Vec<u8> = (0..1024u32).map(|byte| byte as u8).collect();

        let read = copy_entry(&mut content.as_slice(), &mut out, 100).unwrap();

        // The copy reads one byte more than the limit, so the caller knows
        // that the limit is over, and it writes only the bytes that are
        // within the limit.
        assert_eq!(read, 101);
        assert_eq!(fs::metadata(&path).unwrap().len(), 100);
    }

    #[test]
    fn a_script_without_an_extension_is_read_by_its_shebang() {
        assert_eq!(
            catalog::shebang_language("#!/bin/sh\necho hello\n"),
            Some("shell")
        );
        assert_eq!(catalog::shebang_language("#!/bin/bash\n"), Some("shell"));
        assert_eq!(catalog::shebang_language("#!/bin/zsh\n"), Some("shell"));
        assert_eq!(catalog::shebang_language("#!/bin/dash\n"), Some("shell"));
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env python3\n"),
            Some("python")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/node\n"),
            Some("javascript")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/deno\n"),
            Some("javascript")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/bun\n"),
            Some("javascript")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/perl\n"),
            Some("other")
        );
        assert_eq!(catalog::shebang_language("echo hello\n"), None);
    }

    #[test]
    fn a_shebang_that_names_env_names_the_interpreter_behind_its_options() {
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env -S python3\n"),
            Some("python")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env -S python3 -u\n"),
            Some("python")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env -Spython3\n"),
            Some("python")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env -S \"python3 -u\"\n"),
            Some("python")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env -S bash -e\n"),
            Some("shell")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env -S node --loader x\n"),
            Some("javascript")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env LANG=C python3\n"),
            Some("python")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env -u LANG python3\n"),
            Some("python")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env --unset=LANG python3\n"),
            Some("python")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env -i python3\n"),
            Some("python")
        );
        assert_eq!(
            catalog::shebang_language("#!/usr/bin/env -- python3\n"),
            Some("python")
        );
    }

    #[test]
    fn a_file_of_the_window_of_python_is_a_file_of_python() {
        assert_eq!(
            catalog::script_language(Path::new("run.pyw")),
            Some("python")
        );
        assert_eq!(
            catalog::script_language(Path::new("run.py")),
            Some("python")
        );
        assert_eq!(catalog::script_language(Path::new("run.sh")), Some("shell"));
    }

    #[test]
    fn the_stored_copy_is_read_only() {
        let source = skill_fixture("read-only");
        let store = temp_dir("read-only-store");

        let result = add_skill(&options(&source, &store)).unwrap();

        let stored = store.join("skills/demo").join(&result.version.hash);
        assert_eq!(mode_of(&stored.join("run.py")), 0o400);
        assert_eq!(mode_of(&stored.join("SKILL.md")), 0o400);
        assert_eq!(mode_of(&stored), 0o500);
    }

    #[test]
    fn the_store_holds_its_state_in_private_folders() {
        let source = skill_fixture("private-store");
        let store = temp_dir("private-store-store");

        let result = add_skill(&options(&source, &store)).unwrap();

        assert_eq!(mode_of(&store), 0o700);
        assert_eq!(mode_of(&store.join("skills")), 0o700);
        assert_eq!(mode_of(&store.join("skills/demo")), 0o700);
        assert_eq!(mode_of(&store.join("records")), 0o700);
        assert_eq!(mode_of(&store.join("locks")), 0o700);
        assert_eq!(mode_of(&store.join("records/demo.json")), 0o600);
        let stored = store.join("skills/demo").join(&result.version.hash);
        assert_eq!(mode_of(&stored), 0o500);
        assert_eq!(mode_of(&stored.join("run.py")), 0o400);
    }

    #[test]
    fn the_same_skill_is_already_stored() {
        let source = skill_fixture("already-stored");
        let store = temp_dir("already-stored-store");

        let first = add_skill(&options(&source, &store)).unwrap();
        let second = add_skill(&options(&source, &store)).unwrap();

        assert!(!first.already_stored);
        assert!(second.already_stored);
        assert_eq!(first.version.hash, second.version.hash);

        let record = read_record(&store, "demo").unwrap();
        assert_eq!(record.versions.len(), 1);
    }

    #[test]
    fn an_add_of_the_same_content_refreshes_the_record() {
        let source = skill_fixture("refresh");
        let store = temp_dir("refresh-store");

        let first = add_skill(&options(&source, &store)).unwrap();
        assert!(!first.already_stored);
        assert!(!first.refreshed);

        let second = add_skill(&options(&source, &store)).unwrap();
        assert!(second.already_stored);
        assert!(!second.refreshed);
        assert_eq!(first.version.findings, second.version.findings);

        // A record that was written by an older rule holds the wrong
        // findings and the wrong tier.
        let mut record = read_record(&store, "demo").unwrap();
        record.versions[0].findings.clear();
        record.versions[0].tier = Tier::L3;
        record.versions[0].tier_reasons = vec!["an old rule".to_string()];
        let added_at = record.versions[0].added_at.clone();
        let hash = record.versions[0].hash.clone();
        write_record(&store, "demo", &record).unwrap();

        let third = add_skill(&options(&source, &store)).unwrap();

        assert!(third.already_stored);
        assert!(third.refreshed);
        assert_eq!(third.version.tier, Tier::L1);
        assert!(
            third
                .version
                .findings
                .iter()
                .any(|finding| finding.rule == "secret"),
            "{:?}",
            third.version.findings
        );

        let record = read_record(&store, "demo").unwrap();
        assert_eq!(record.versions.len(), 1);
        assert_eq!(record.versions[0].hash, hash);
        assert_eq!(record.versions[0].added_at, added_at);
        assert_eq!(record.versions[0].tier, Tier::L1);
        assert!(
            record.versions[0]
                .tier_reasons
                .iter()
                .any(|reason| reason.contains("os")),
            "{:?}",
            record.versions[0].tier_reasons
        );
        assert!(
            record.versions[0]
                .findings
                .iter()
                .any(|finding| finding.rule == "secret"),
            "{:?}",
            record.versions[0].findings
        );
    }

    #[test]
    fn an_add_of_the_same_content_refreshes_the_scripts() {
        let source = skill_fixture("refresh-scripts");
        let store = temp_dir("refresh-scripts-store");

        let first = add_skill(&options(&source, &store)).unwrap();
        assert_eq!(first.version.scripts.len(), 1);
        assert_eq!(first.version.scripts[0].path, "run.py");
        assert_eq!(first.version.scripts[0].imports, vec!["os".to_string()]);

        // A record that was written by an older rule holds no scripts.
        let mut record = read_record(&store, "demo").unwrap();
        record.versions[0].scripts.clear();
        write_record(&store, "demo", &record).unwrap();

        let second = add_skill(&options(&source, &store)).unwrap();

        assert!(second.already_stored);
        assert!(second.refreshed);
        assert_eq!(second.version.scripts, first.version.scripts);

        let record = read_record(&store, "demo").unwrap();
        assert_eq!(record.versions.len(), 1);
        assert_eq!(record.versions[0].scripts, first.version.scripts);
    }

    #[test]
    fn a_changed_file_adds_a_second_version() {
        let source = skill_fixture("second-version");
        let store = temp_dir("second-version-store");

        let first = add_skill(&options(&source, &store)).unwrap();
        write(&source.join("run.py"), "import os\n\nprint('hello')\n");
        let second = add_skill(&options(&source, &store)).unwrap();

        assert!(!second.already_stored);
        assert_ne!(first.version.hash, second.version.hash);

        let record = read_record(&store, "demo").unwrap();
        assert_eq!(record.versions.len(), 2);
        assert_eq!(record.versions[0].hash, first.version.hash);
        assert_eq!(record.versions[1].hash, second.version.hash);
        assert!(record.versions[1].findings.is_empty());
    }

    #[test]
    fn a_symlink_is_a_finding_and_is_never_copied() {
        let source = skill_fixture("symlink");
        let store = temp_dir("symlink-store");
        std::os::unix::fs::symlink("/etc/passwd", source.join("link.txt")).unwrap();

        let result = add_skill(&options(&source, &store)).unwrap();

        let finding = result
            .version
            .findings
            .iter()
            .find(|finding| finding.rule == "symlink")
            .expect("a symlink finding");
        assert_eq!(finding.path, "link.txt");
        assert_eq!(finding.severity, Severity::Medium);
        assert!(
            !store
                .join("skills/demo")
                .join(&result.version.hash)
                .join("link.txt")
                .exists()
        );
    }

    #[test]
    fn a_zip_with_a_parent_path_is_refused() {
        let store = temp_dir("zip-parent-store");
        let archive = temp_dir("zip-parent").join("evil.zip");
        write_zip(&archive, &[("../evil", "boom")]);

        let error = add_skill(&options(&archive, &store)).unwrap_err();

        assert!(
            error.to_string().contains("leaves the archive root"),
            "{error}"
        );
        assert!(!store.join("skills").exists());
        assert!(!store.join("records").exists());
    }

    #[test]
    fn a_zip_with_a_symlink_is_refused() {
        let store = temp_dir("zip-symlink-store");
        let archive = temp_dir("zip-symlink").join("link.zip");
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

        let error = add_skill(&options(&archive, &store)).unwrap_err();

        assert!(error.to_string().contains("is a symlink"), "{error}");
        assert!(!store.join("skills").exists());
    }

    #[test]
    fn a_zip_with_too_many_entries_is_refused() {
        let store = temp_dir("zip-many-store");
        let archive = temp_dir("zip-many").join("many.zip");
        let file = fs::File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let entry_options =
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for index in 0..=MAX_ARCHIVE_ENTRIES {
            writer
                .start_file(format!("file-{index}.txt"), entry_options)
                .unwrap();
            writer.write_all(b"x").unwrap();
        }
        writer.finish().unwrap();

        let error = add_skill(&options(&archive, &store)).unwrap_err();

        assert!(error.to_string().contains("the limit is"), "{error}");
        assert!(!store.join("skills").exists());
    }

    #[test]
    fn a_zip_with_one_top_folder_is_rooted_at_that_folder() {
        let store = temp_dir("zip-folder-store");
        let archive = temp_dir("zip-folder").join("zipped.zip");
        write_zip(
            &archive,
            &[
                ("my-skill/SKILL.md", "---\nname: zipped\n---\n\n# Zipped\n"),
                ("my-skill/run.py", "import sys\n"),
            ],
        );

        let result = add_skill(&options(&archive, &store)).unwrap();

        assert_eq!(result.id, "zipped");
        assert_eq!(result.version.name.as_deref(), Some("zipped"));
        assert_eq!(result.version.files, 2);
        assert!(
            store
                .join("skills/zipped")
                .join(&result.version.hash)
                .join("SKILL.md")
                .is_file()
        );
    }

    #[test]
    fn the_id_comes_from_the_source_and_not_from_the_frontmatter_name() {
        let root = temp_dir("two-skills");
        let first = root.join("a-skill");
        let second = root.join("b-skill");
        for source in [&first, &second] {
            write(
                &source.join("SKILL.md"),
                "---\nname: same\ndescription: A shared name.\n---\n\n# Same\n",
            );
        }
        let store = temp_dir("two-skills-store");

        let one = add_skill(&options(&first, &store)).unwrap();
        let two = add_skill(&options(&second, &store)).unwrap();

        assert_eq!(one.id, "a-skill");
        assert_eq!(two.id, "b-skill");
        assert_eq!(one.version.name.as_deref(), Some("same"));
        assert_eq!(two.version.name.as_deref(), Some("same"));
        assert!(store.join("records/a-skill.json").is_file());
        assert!(store.join("records/b-skill.json").is_file());
    }

    #[test]
    fn a_source_with_a_valid_name_wins_over_an_invalid_frontmatter_name() {
        let source = temp_dir("good-id-source").join("good-id");
        write(
            &source.join("SKILL.md"),
            "---\nname: \u{547d}\u{7406}\u{5927}\u{5e08}\n---\n\n# Master\n",
        );
        let store = temp_dir("good-id-store");

        let result = add_skill(&options(&source, &store)).unwrap();

        assert_eq!(result.id, "good-id");
        assert_eq!(
            result.version.name.as_deref(),
            Some("\u{547d}\u{7406}\u{5927}\u{5e08}")
        );
        assert!(store.join("records/good-id.json").is_file());
    }

    #[test]
    fn a_zip_is_stored_under_the_stem_of_its_name() {
        let store = temp_dir("zip-stem-store");
        let archive = temp_dir("zip-stem").join("x.zip");
        write_zip(
            &archive,
            &[("SKILL.md", "---\nname: other\n---\n\n# Other\n")],
        );

        let result = add_skill(&options(&archive, &store)).unwrap();

        assert_eq!(result.id, "x");
        assert_eq!(result.version.name.as_deref(), Some("other"));
        assert!(store.join("records/x.json").is_file());
    }

    #[test]
    fn a_source_and_a_frontmatter_name_that_are_no_ids_ask_for_an_id() {
        let source = temp_dir("bad-id-source").join("\u{547d}\u{7406}\u{5927}\u{5e08}");
        write(
            &source.join("SKILL.md"),
            "---\nname: \u{547d}\u{7406}\u{5927}\u{5e08}\n---\n\n# Master\n",
        );
        let store = temp_dir("bad-id-store");

        let error = add_skill(&options(&source, &store)).unwrap_err();

        assert!(error.to_string().contains("--id"), "{error}");
        assert!(!store.join("records").exists());
    }

    #[test]
    fn the_named_id_wins_over_the_source_and_the_frontmatter_name() {
        let source = temp_dir("named-id-source").join("a-skill");
        write(
            &source.join("SKILL.md"),
            "---\nname: same\ndescription: A shared name.\n---\n\n# Same\n",
        );
        let store = temp_dir("named-id-store");

        let result = add_skill(&AddOptions {
            source,
            store: Some(store.clone()),
            id: Some("named-id".to_string()),
            from: None,
        })
        .unwrap();

        assert_eq!(result.id, "named-id");
        assert!(store.join("records/named-id.json").is_file());
    }

    #[test]
    fn a_file_with_elf_magic_is_a_binary_finding() {
        let source = skill_fixture("binary");
        let store = temp_dir("binary-store");
        fs::write(source.join("tool.bin"), b"\x7fELF\x02\x01\x01\x00rest").unwrap();

        let result = add_skill(&options(&source, &store)).unwrap();

        let finding = result
            .version
            .findings
            .iter()
            .find(|finding| finding.rule == "binary")
            .expect("a binary finding");
        assert_eq!(finding.path, "tool.bin");
        assert_eq!(finding.severity, Severity::High);
        assert_eq!(finding.line, None);
    }

    #[test]
    fn the_list_shows_the_newest_hash_and_the_counts() {
        let source = skill_fixture("list");
        let store = temp_dir("list-store");

        let result = add_skill(&options(&source, &store)).unwrap();

        let summaries = list_records(&store).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, "demo");
        assert_eq!(summaries[0].hash, result.version.hash);
        assert_eq!(summaries[0].tier, "L1");
        assert_eq!(summaries[0].high, 0);
        assert_eq!(summaries[0].medium, 1);
        assert_eq!(summaries[0].low, 0);

        let text = render_summaries(&summaries);
        assert!(text.contains("demo"), "{text}");
        assert!(text.contains("tier L1"), "{text}");
        assert!(!text.contains(SECRET), "{text}");
    }

    #[test]
    fn a_named_store_root_wins_over_the_environment() {
        let source = skill_fixture("environment");
        let store = temp_dir("environment-store");

        let result = add_skill(&options(&source, &store)).unwrap();

        assert!(store.join("records/demo.json").is_file());
        assert_eq!(result.id, "demo");
        assert_eq!(
            resolve_store_root(Some(&store)).unwrap(),
            store.to_path_buf()
        );
    }

    #[test]
    fn an_unknown_id_has_no_record() {
        let store = temp_dir("unknown-store");

        let error = read_record(&store, "nothing").unwrap_err();

        assert!(error.to_string().contains("no stored skill"), "{error}");
    }

    #[test]
    fn the_record_holds_the_provenance_and_the_time() {
        let source = skill_fixture("provenance");
        let store = temp_dir("provenance-store");

        let result = add_skill(&AddOptions {
            source,
            store: Some(store.clone()),
            id: None,
            from: Some("https://example.com/repo".to_string()),
        })
        .unwrap();

        assert_eq!(
            result.version.from.as_deref(),
            Some("https://example.com/repo")
        );
        assert!(
            result.version.added_at.ends_with('Z'),
            "{:?}",
            result.version.added_at
        );

        let record = read_record(&store, "demo").unwrap();
        assert_eq!(record.schema_version, RECORD_SCHEMA_VERSION);
        assert_eq!(record.id, "demo");
        assert_eq!(record.versions.len(), 1);
    }

    #[test]
    fn a_zip_above_the_size_limit_is_refused() {
        let dir = temp_dir("zip-size");
        let archive = dir.join("big.zip");
        let file = fs::File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file("big.txt", options).unwrap();
        writer.write_all(&[b'x'; 512]).unwrap();
        writer.finish().unwrap();

        let error =
            extract_zip_with_limits(&archive, &dir.join("out"), 100, MAX_ARCHIVE_ENTRIES, 100)
                .unwrap_err();

        assert!(error.to_string().contains("more than 100 bytes"), "{error}");
    }

    #[test]
    fn a_zip_above_the_entry_limit_is_refused() {
        let dir = temp_dir("zip-entries");
        let archive = dir.join("many.zip");
        write_zip(
            &archive,
            &[("one.txt", "a"), ("two.txt", "b"), ("three.txt", "c")],
        );

        let error = extract_zip_with_limits(&archive, &dir.join("out"), MAX_ARCHIVE_BYTES, 2, 100)
            .unwrap_err();

        assert!(error.to_string().contains("the limit is 2"), "{error}");
    }

    #[test]
    fn a_zip_entry_above_the_ratio_limit_is_refused() {
        let dir = temp_dir("zip-ratio");
        let archive = dir.join("ratio.zip");
        let file = fs::File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer.start_file("zeros.txt", options).unwrap();
        writer.write_all(&[b'a'; 4096]).unwrap();
        writer.finish().unwrap();

        let error = extract_zip_with_limits(&archive, &dir.join("out"), MAX_ARCHIVE_BYTES, 100, 10)
            .unwrap_err();

        assert!(
            error.to_string().contains("compression ratio above 10"),
            "{error}"
        );
    }

    #[test]
    fn a_zip_path_that_leaves_the_root_is_refused() {
        assert!(safe_relative_path("../evil").is_err());
        assert!(safe_relative_path("/etc/passwd").is_err());
        assert!(safe_relative_path("a/../../evil").is_err());
        assert_eq!(safe_relative_path("a/../b").unwrap(), PathBuf::from("b"));
    }

    #[test]
    fn a_source_that_is_a_symlink_is_refused() {
        let source = skill_fixture("symlink-source");
        let store = temp_dir("symlink-source-store");
        let link = temp_dir("symlink-source-link").join("link");
        std::os::unix::fs::symlink(&source, &link).unwrap();

        let error = add_skill(&options(&link, &store)).unwrap_err();

        assert!(error.to_string().contains("is a symlink"), "{error}");
        assert!(!store.join("skills").exists());
        assert!(!store.join("records").exists());
    }

    #[test]
    fn a_source_with_a_single_top_level_folder_is_rooted_at_that_folder() {
        let source = temp_dir("single-folder").join("outer");
        write(
            &source.join("my-skill/SKILL.md"),
            "---\nname: nested\n---\n\n# Nested\n",
        );
        write(&source.join("my-skill/run.py"), "import sys\n");
        let store = temp_dir("single-folder-store");

        let result = add_skill(&options(&source, &store)).unwrap();

        assert_eq!(result.id, "outer");
        assert_eq!(result.version.name.as_deref(), Some("nested"));
        assert_eq!(result.version.files, 2);
    }

    #[test]
    fn a_source_with_more_than_one_top_level_entry_is_refused() {
        let source = temp_dir("two-entries");
        write(
            &source.join("my-skill/SKILL.md"),
            "---\nname: nested\n---\n\n# Nested\n",
        );
        write(&source.join("README.md"), "# Readme\n");
        let store = temp_dir("two-entries-store");

        let error = add_skill(&options(&source, &store)).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("SKILL.md must be at the top level or inside a single top-level folder"),
            "{error}"
        );
        assert!(!store.join("skills").exists());
        assert!(!store.join("records").exists());
    }

    #[test]
    fn a_secret_in_a_file_with_bytes_that_are_not_text_is_found() {
        let source = skill_fixture("binary-secret");
        let store = temp_dir("binary-secret-store");
        let mut content = b"\x00\x01\x02payload\n".to_vec();
        content.extend_from_slice(format!("KEY = \"{SECRET}\"\n").as_bytes());
        fs::write(source.join("payload.dat"), &content).unwrap();

        let result = add_skill(&options(&source, &store)).unwrap();

        let finding = result
            .version
            .findings
            .iter()
            .find(|finding| finding.rule == "secret")
            .expect("a secret finding");
        assert_eq!(finding.path, "payload.dat");
        assert_eq!(finding.line, Some(2));
        assert_eq!(finding.severity, Severity::Medium);
    }

    #[test]
    fn a_secret_next_to_bytes_that_are_not_utf8_is_found() {
        let source = skill_fixture("lossy-secret");
        let store = temp_dir("lossy-secret-store");
        let mut content = vec![0xff, 0xfe, b'\n'];
        content.extend_from_slice(format!("token = \"{SECRET}\"\n").as_bytes());
        fs::write(source.join("notes.txt"), &content).unwrap();

        let result = add_skill(&options(&source, &store)).unwrap();

        let finding = result
            .version
            .findings
            .iter()
            .find(|finding| finding.rule == "secret")
            .expect("a secret finding");
        assert_eq!(finding.path, "notes.txt");
        assert_eq!(finding.line, Some(2));
    }

    #[test]
    fn a_stored_copy_without_a_version_in_the_record_repairs_the_record() {
        let source = skill_fixture("repair");
        let store = temp_dir("repair-store");

        let first = add_skill(&options(&source, &store)).unwrap();
        fs::remove_file(store.join("records/demo.json")).unwrap();

        let second = add_skill(&options(&source, &store)).unwrap();

        assert!(!second.already_stored);
        assert_eq!(second.version.hash, first.version.hash);
        let record = read_record(&store, "demo").unwrap();
        assert_eq!(record.versions.len(), 1);
        assert_eq!(record.versions[0].hash, first.version.hash);
    }

    #[test]
    fn the_lock_of_a_skill_id_is_taken_and_released() {
        let store = temp_dir("lock-store");

        let lock = Lock::acquire(&store, "demo").unwrap();
        assert!(store.join("locks/demo.lock").is_file());

        let error = Lock::acquire_within(&store, "demo", Duration::from_millis(100)).unwrap_err();
        assert!(
            error.to_string().contains("is locked by another add"),
            "{error}"
        );

        drop(lock);
        assert!(!store.join("locks/demo.lock").exists());

        let again = Lock::acquire(&store, "demo").unwrap();
        drop(again);
        assert!(!store.join("locks/demo.lock").exists());
    }

    #[test]
    fn a_failing_add_leaves_no_lock_behind() {
        let source = skill_fixture("lock-failure");
        let store = temp_dir("lock-failure-store");
        fs::write(store.join("skills"), "not a folder").unwrap();

        let error = add_skill(&options(&source, &store)).unwrap_err();

        assert!(error.to_string().contains("skills"), "{error}");
        assert!(!store.join("locks/demo.lock").exists());
    }

    #[test]
    fn an_add_leaves_no_lock_and_no_temporary_file_behind() {
        let source = skill_fixture("no-leftovers");
        let store = temp_dir("no-leftovers-store");

        add_skill(&options(&source, &store)).unwrap();

        assert!(!store.join("locks/demo.lock").exists());
        let names: Vec<String> = fs::read_dir(store.join("records"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["demo.json".to_string()]);
        let stored: Vec<String> = fs::read_dir(store.join("skills/demo"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            stored.iter().all(|name| !name.starts_with(".incoming-")),
            "{stored:?}"
        );
    }

    #[test]
    fn a_name_that_is_not_utf8_is_refused() {
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"bad-\xff-name"));

        let error = utf8_name(&path, "entry").unwrap_err();

        assert!(error.to_string().contains("not valid UTF-8"), "{error}");
        assert!(
            error.to_string().contains('\u{fffd}'),
            "the name is shown lossily: {error}"
        );
    }

    #[test]
    fn a_zip_entry_with_a_name_that_is_not_utf8_is_refused() {
        let store = temp_dir("utf8-zip-store");
        let archive = temp_dir("utf8-zip").join("bad.zip");
        write_zip_with_raw_name(&archive, b"bad-\xff-name.txt", b"hello\n");

        let error = add_skill(&options(&archive, &store)).unwrap_err();

        assert!(error.to_string().contains("not valid UTF-8"), "{error}");
        assert!(!store.join("skills").exists());
        assert!(!store.join("records").exists());
    }

    #[test]
    fn every_string_of_a_record_is_redacted() {
        let record = ThreatRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            id: SECRET.to_string(),
            versions: vec![StoredVersion {
                hash: SECRET.to_string(),
                full_hash: SECRET.to_string(),
                from: Some(SECRET.to_string()),
                added_at: "2026-01-01T00:00:00Z".to_string(),
                files: 1,
                bytes: 1,
                name: Some(SECRET.to_string()),
                description: Some(SECRET.to_string()),
                scripts: vec![StoredScript {
                    path: SECRET.to_string(),
                    language: SECRET.to_string(),
                    imports: vec![SECRET.to_string()],
                }],
                findings: vec![Finding {
                    rule: SECRET.to_string(),
                    severity: Severity::High,
                    path: SECRET.to_string(),
                    line: Some(1),
                    detail: Some(SECRET.to_string()),
                }],
                tier: Tier::None,
                tier_reasons: vec![SECRET.to_string()],
            }],
        };

        let text = redacted_record_json(&record).unwrap();
        assert!(!text.contains(SECRET), "{text}");
        assert!(text.contains("<redacted>"), "{text}");

        let rendered = render_record(&record);
        assert!(!rendered.contains(SECRET), "{rendered}");
    }

    #[test]
    fn the_lock_holds_the_pid_of_the_process_that_took_it() {
        let dir = temp_dir("lock-pid");
        let lock = Lock::acquire_within(&dir, "demo", Duration::from_secs(1)).unwrap();

        let content = fs::read_to_string(&lock.path).unwrap();
        assert_eq!(content.trim(), std::process::id().to_string());
    }

    #[test]
    fn a_lock_of_a_process_that_is_gone_is_removed() {
        let dir = temp_dir("lock-gone");
        let path = dir.join("demo.lock");

        // A lock of a process that is gone is stale, so it is removed and
        // the add goes on.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        write(&path, &format!("{}\n", child.id()));
        assert!(remove_stale_lock(&path));
        assert!(!path.exists());
    }

    #[test]
    fn a_path_that_names_another_file_is_no_file() {
        let dir = temp_dir("lock-same-file");
        let path = dir.join("demo.lock");
        write(&path, "1\n");
        let file = fs::File::options()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();

        // The path names the file that was opened.
        assert!(same_file(&file, &path));

        // Another file took the place of the lock, so the path names
        // another file and the lock of the file is no lock of the path.
        let other = dir.join("other.lock");
        write(&other, "2\n");
        fs::rename(&other, &path).unwrap();
        assert!(!same_file(&file, &path));
    }

    #[test]
    fn a_lock_that_holds_no_pid_is_no_stale_lock() {
        let dir = temp_dir("lock-no-pid");
        let path = dir.join("demo.lock");

        // A lock that holds no pid is no lock of a known process, so it is
        // only stale when it is old.
        write(&path, "not a pid\n");
        assert!(!remove_stale_lock(&path));
        assert!(path.exists());
    }

    #[test]
    fn a_young_lock_that_a_process_holds_is_no_stale_lock() {
        let dir = temp_dir("lock-held");
        let lock = Lock::acquire_within(&dir, "demo", Duration::from_secs(1)).unwrap();

        // The process holds the lock of the file, so the process works on
        // the skill and its lock is no stale lock.
        assert!(!remove_stale_lock(&lock.path));
        assert!(lock.path.exists());
    }

    #[test]
    fn an_old_lock_is_stale_even_when_a_process_holds_it() {
        let dir = temp_dir("lock-old");
        let lock = Lock::acquire_within(&dir, "demo", Duration::from_secs(1)).unwrap();

        // The lock is old, so the add that took it never finished and the
        // lock is stale, even when a process holds the lock of the file.
        let file = fs::File::options().write(true).open(&lock.path).unwrap();
        let old = SystemTime::now() - Duration::from_secs(20 * 60);
        file.set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();

        assert!(remove_stale_lock(&lock.path));
        assert!(!lock.path.exists());
    }

    #[test]
    fn the_release_of_a_lock_leaves_the_lock_of_another_process_alone() {
        let dir = temp_dir("lock-release");
        let lock = Lock::acquire_within(&dir, "demo", Duration::from_secs(1)).unwrap();
        let path = lock.path.clone();

        // Another add took the lock in the meantime, so the file of the
        // path is another file than the file of the lock.
        let other = dir.join("other.lock");
        write(&other, "12345\n");
        fs::rename(&other, &path).unwrap();

        drop(lock);

        // The lock of the other add stays, so the two adds never write the
        // same record at the same time.
        assert!(path.exists());
    }

    /// A stored version with the given tier and findings.
    fn stored_version(tier: Tier, findings: Vec<Finding>) -> StoredVersion {
        StoredVersion {
            hash: "0123456789abcdef".to_string(),
            full_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
            from: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
            files: 2,
            bytes: 42,
            name: Some("demo".to_string()),
            description: None,
            scripts: Vec::new(),
            findings,
            tier,
            tier_reasons: Vec::new(),
        }
    }

    #[test]
    fn a_path_that_leaves_the_version_is_refused() {
        assert!(safe_relative("SKILL.md").is_ok());
        assert!(safe_relative("docs/guide.md").is_ok());
        assert!(safe_relative("./docs/guide.md").is_ok());
        assert!(safe_relative("../SKILL.md").is_err());
        assert!(safe_relative("docs/../../SKILL.md").is_err());
        assert!(safe_relative("/etc/passwd").is_err());
        assert!(safe_relative("").is_err());
    }

    #[test]
    fn a_relative_path_becomes_an_absolute_path() {
        let absolute = absolute_path(Path::new("store/skills/demo"));

        assert!(absolute.is_absolute(), "{absolute:?}");
    }

    #[test]
    fn a_version_of_the_tier_l3_is_not_safe() {
        let version = stored_version(Tier::L3, Vec::new());

        let reason = unsafe_reason(&version).expect("the reason of the refusal");
        assert!(reason.contains("L3"), "{reason}");
        assert!(refuse_unsafe_version(&version).is_err());
    }

    #[test]
    fn a_version_with_a_high_finding_is_not_safe() {
        let version = stored_version(
            Tier::L1,
            vec![Finding {
                rule: "binary".to_string(),
                severity: Severity::High,
                path: "tool.bin".to_string(),
                line: None,
                detail: None,
            }],
        );

        let reason = unsafe_reason(&version).expect("the reason of the refusal");
        assert!(reason.contains("high finding"), "{reason}");
        assert!(reason.contains("binary"), "{reason}");
        assert!(refuse_unsafe_version(&version).is_err());
    }

    #[test]
    fn a_version_without_a_high_finding_is_safe() {
        let version = stored_version(Tier::L1, Vec::new());

        assert!(unsafe_reason(&version).is_none());
        assert!(refuse_unsafe_version(&version).is_ok());
    }

    #[test]
    fn a_safe_search_keeps_only_the_versions_that_need_no_container() {
        let source = temp_dir("safe-search-container").join("container");
        write(
            &source.join("SKILL.md"),
            "---\nname: container\ndescription: A widget that needs a container.\n---\n\n# Container\n",
        );
        write(
            &source.join("run.py"),
            "import requests\n\nprint(requests)\n",
        );
        let store = temp_dir("safe-search-store");
        let added = add_skill(&options(&source, &store)).unwrap();

        // The skill needs a container and holds no high finding at all, so
        // it is a result of a search without a filter.
        assert_eq!(added.version.tier, Tier::L2);
        assert_eq!(count_findings(&added.version, Severity::High), 0);

        let mut search = StoreSearchOptions::new("widget");
        let hits = search_store(&store, &search).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].tier, Tier::L2);

        // A safe search keeps the versions that need no container, so the
        // skill is never a result of a safe search.
        search.safe = true;
        let hits = search_store(&store, &search).unwrap();
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn a_file_of_a_stored_version_is_read() {
        let source = skill_fixture("read-version");
        let store = temp_dir("read-version-store");
        let added = add_skill(&options(&source, &store)).unwrap();

        let location = version_location(&store, "demo", None).unwrap();
        let text = read_version_file(&location, SKILL_FILE).unwrap();
        assert!(text.contains("# Demo"), "{text}");

        let named = version_location(&store, "demo", Some(&added.version.hash)).unwrap();
        assert_eq!(read_version_file(&named, SKILL_FILE).unwrap(), text);

        let location = version_location(&store, "demo", None).unwrap();
        assert!(read_version_file(&location, "../SKILL.md").is_err());
        assert!(read_version_file(&location, "/etc/passwd").is_err());

        let location = version_location(&store, "demo", None).unwrap();
        let files = list_version_markdown(&location).unwrap();
        assert_eq!(files, vec![SKILL_FILE.to_string()]);
    }

    #[test]
    fn an_empty_version_names_no_version() {
        let source = skill_fixture("empty-version");
        let store = temp_dir("empty-version-store");
        add_skill(&options(&source, &store)).unwrap();

        // A version that is named but holds no name is not the newest
        // version, so the command fails.
        assert!(version_location(&store, "demo", Some("")).is_err());
        assert!(version_location(&store, "demo", Some("   ")).is_err());
        assert!(version_location(&store, "demo", None).is_ok());
    }

    #[test]
    fn a_version_without_a_folder_is_not_in_the_store() {
        let source = skill_fixture("gone-version");
        let store = temp_dir("gone-version-store");
        let added = add_skill(&options(&source, &store)).unwrap();

        let location = version_location(&store, "demo", None).unwrap();
        assert!(stored_folder(&location).is_ok());

        // The folder of the version is gone, so the version is not a
        // version of the store any more. The copy in the store is read
        // only, so the folder is opened before it is removed.
        let mut permissions = fs::metadata(&location.path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&location.path, permissions).unwrap();
        fs::remove_dir_all(&location.path).unwrap();
        assert!(stored_folder(&location).is_err());
        assert!(read_version_file(&location, SKILL_FILE).is_err());
        assert!(list_version_markdown(&location).is_err());

        let _ = added;
    }

    #[test]
    fn a_file_of_a_version_is_never_read_through_a_link() {
        let source = skill_fixture("link-version");
        let store = temp_dir("link-version-store");
        add_skill(&options(&source, &store)).unwrap();

        let location = version_location(&store, "demo", None).unwrap();
        let folder = location.path.clone();
        let mut permissions = fs::metadata(&folder).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&folder, permissions).unwrap();

        // A folder of the version that is a link is never read through.
        let outside = temp_dir("link-version-outside");
        write(&outside.join("secret.md"), "# Secret\n");
        std::os::unix::fs::symlink(&outside, folder.join("docs")).unwrap();

        let error = read_version_file(&location, "docs/secret.md").unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("symlink"), "{message}");
        assert!(!message.contains("Secret"), "{message}");
    }

    #[test]
    fn a_skill_folder_that_is_a_link_is_never_read_through() {
        let source = skill_fixture("link-skill-folder");
        let store = temp_dir("link-skill-folder-store");
        add_skill(&options(&source, &store)).unwrap();

        let location = version_location(&store, "demo", None).unwrap();

        // The folder of the skill is put in the place of a link that points
        // at a folder that holds the very same version, so the file behind
        // the link is not a file of the store.
        let skills = store.join("skills");
        let outside = temp_dir("link-skill-folder-outside");
        write(&outside.join(&location.hash).join(SKILL_FILE), "# Secret\n");
        make_writable(&skills.join("demo"));
        fs::remove_dir_all(skills.join("demo")).unwrap();
        std::os::unix::fs::symlink(&outside, skills.join("demo")).unwrap();

        // The file behind the link is never read.
        let error = read_version_file(&location, SKILL_FILE).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("symlink"), "{message}");
        assert!(!message.contains("Secret"), "{message}");

        // The files behind the link are never listed either.
        let error = list_version_markdown(&location).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("symlink"), "{message}");
        assert!(!message.contains("Secret"), "{message}");
    }

    #[test]
    fn the_files_of_a_version_are_opened_through_its_descriptor() {
        let source = skill_fixture("descriptor-version");
        let store = temp_dir("descriptor-version-store");
        add_skill(&options(&source, &store)).unwrap();

        let location = version_location(&store, "demo", None).unwrap();
        let folder = location.path.clone();
        let mut permissions = fs::metadata(&folder).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&folder, permissions).unwrap();

        // The folder of the version is opened before the link is put in
        // the place of the folder of the guide, so the link is seen and
        // the file behind it is never read.
        let directory = open_version_dir(&location).unwrap();
        let guide = std::ffi::CString::new("docs").unwrap();
        fs::create_dir_all(folder.join("docs")).unwrap();
        write(&folder.join("docs/guide.md"), "# Guide\n");
        let outside = temp_dir("descriptor-version-outside");
        write(&outside.join("secret.md"), "# Secret\n");
        fs::remove_dir_all(folder.join("docs")).unwrap();
        std::os::unix::fs::symlink(&outside, folder.join("docs")).unwrap();

        let error = open_relative(
            directory.as_raw_fd(),
            &guide,
            true,
            &location,
            Path::new("docs/guide.md"),
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("symlink"), "{message}");
        assert!(!message.contains("Secret"), "{message}");
    }

    #[test]
    fn the_list_of_a_version_is_read_through_its_descriptor() {
        let source = skill_fixture("descriptor-list");
        let store = temp_dir("descriptor-list-store");
        add_skill(&options(&source, &store)).unwrap();

        let location = version_location(&store, "demo", None).unwrap();
        let folder = location.path.clone();
        let mut permissions = fs::metadata(&folder).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&folder, permissions).unwrap();

        // The folder of the version is opened before it is moved away and
        // put in the place of a link, so the list is the list of the
        // version and never the list of the folder that the link points
        // at.
        let directory = open_version_dir(&location).unwrap();
        let outside = temp_dir("descriptor-list-outside");
        write(&outside.join("secret.md"), "# Secret\n");
        let moved = folder.with_extension("moved");
        fs::rename(&folder, &moved).unwrap();
        std::os::unix::fs::symlink(&outside, &folder).unwrap();

        let mut files = Vec::new();
        collect_markdown_at(directory.as_raw_fd(), "", &mut files, &location).unwrap();
        assert_eq!(files, vec![SKILL_FILE.to_string()]);
    }

    #[test]
    fn a_lock_of_a_process_that_is_gone_is_stale() {
        let dir = temp_dir("lock-stale");
        let path = dir.join("demo.lock");

        // A lock of a process that is gone is stale.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        let dead = child.id().to_string();
        write(&path, &format!("{dead}\n"));
        assert!(stale_lock(&path, &format!("{dead}\n")));

        // A lock of this process is not stale.
        let alive = std::process::id().to_string();
        write(&path, &format!("{alive}\n"));
        assert!(!stale_lock(&path, &format!("{alive}\n")));

        // A lock that is older than ten minutes is stale, whatever its pid
        // holds.
        write(&path, &format!("{alive}\n"));
        let file = fs::File::options().write(true).open(&path).unwrap();
        let old = SystemTime::now() - Duration::from_secs(20 * 60);
        file.set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();
        assert!(stale_lock(&path, &format!("{alive}\n")));
    }
}
