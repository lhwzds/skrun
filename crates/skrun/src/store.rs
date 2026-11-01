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
//! - refusal of an unsafe zip archive
//! - refusal of a source that is a symlink
//! - refusal of a name that is not text
//! - the id of an added skill, taken from the real path of its source
//! - the content hash of a staged skill
//! - the name of the folder of every version of a skill
//! - the read-only copy of a stored skill
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
//! - reach the network
//! - overwrite a stored version
//! - print a value that looks like a secret
//! - skip a file of the scan because it is not text
//!
//! ## Inputs
//! - a local skill directory or a zip archive
//! - a store root directory
//!
//! ## Outputs
//! - a read-only stored skill copy
//! - a threat record
//! - a store summary
//!
//! ## Depends On
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
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Schema version written into every threat record.
pub const RECORD_SCHEMA_VERSION: u32 = 1;

/// Environment variable that holds the store root.
pub const STORE_ENV: &str = "SKRUN_STORE";

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
/// the findings of today. The hash, the source and the time of the version
/// stay the same, because the content did not change. A stored copy whose
/// version is missing from the record repairs the record instead.
pub fn add_skill(options: &AddOptions) -> Result<AddResult> {
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
    create_private_dir(&skills_root.join(&id))?;

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
        // refresh. The hash, the source and the time of the version stay the
        // same, because the content did not change.
        let stored = &mut record.versions[index];
        let refreshed = stored.scripts != version.scripts
            || stored.findings != version.findings
            || stored.tier != version.tier
            || stored.tier_reasons != version.tier_reasons;
        if refreshed {
            stored.scripts = version.scripts.clone();
            stored.findings = version.findings.clone();
            stored.tier = version.tier;
            stored.tier_reasons = version.tier_reasons.clone();
            write_record(&root, &id, &record)?;
        }
        let stored = record.versions[index].clone();
        return Ok(AddResult {
            id,
            already_stored: true,
            refreshed,
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
    let directory = root.join("records");
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut summaries = Vec::new();
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
    summaries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(summaries)
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
        for attempt in 0..64u32 {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "skrun-store-{}-{stamp}-{attempt}",
                std::process::id()
            ));
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
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

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
        let root = std::env::temp_dir().join(format!(
            "skrun-store-test-{name}-{}-{now}",
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
