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
//! - the read-only copy of a stored skill
//! - the threat record of every added version
//! - the lock of a skill id while it is added
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
//! - serde_json
//! - sha2
//! - zip
//!
//! ## Verify
//! - cargo test -p skrun

use crate::catalog;
use crate::runtime::validate_id;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
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

/// How long an add waits for the lock of a skill id.
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// How long an add waits between two tries for the lock of a skill id.
const LOCK_RETRY: Duration = Duration::from_millis(50);

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
    /// A value that looks like a secret.
    High,
    /// A payload that does not belong in a text skill.
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
    /// The short content hash that names the version.
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
/// a second time and no second version is written. A stored copy whose
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

    let (hash, full_hash, files, bytes) = content_hash(&skill_root)?;
    let (scripts, mut findings) = scan_skill(&skill_root)?;
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
        });
    }
    findings.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.line.cmp(&right.line))
    });

    let version = StoredVersion {
        hash: hash.clone(),
        full_hash,
        from: options.from.as_deref().map(catalog::redact),
        added_at: rfc3339(SystemTime::now()),
        files,
        bytes,
        name: frontmatter.name.as_deref().map(catalog::redact),
        description: frontmatter.description.as_deref().map(catalog::redact),
        scripts,
        findings,
    };

    let skills_root = root.join("skills");
    let target = skills_root.join(&id).join(&hash);
    if target.exists() {
        let mut record = load_record(&root, &id)?;
        if let Some(stored) = record
            .versions
            .iter()
            .find(|stored| stored.hash == version.hash)
            .cloned()
        {
            return Ok(AddResult {
                id,
                already_stored: true,
                version: stored,
            });
        }
        // The copy of the version is there, but the record lost it: the
        // record is repaired instead of the version being stored again.
        record.versions.push(version.clone());
        write_record(&root, &id, &record)?;
        return Ok(AddResult {
            id,
            already_stored: false,
            version,
        });
    }

    fs::create_dir_all(skills_root.join(&id))
        .with_context(|| format!("create `{}`", catalog::redact_path(&skills_root)))?;
    store_copy(&skill_root, &skills_root.join(&id), &target)?;

    let mut record = load_record(&root, &id)?;
    if !record
        .versions
        .iter()
        .any(|stored| stored.hash == version.hash)
    {
        record.versions.push(version.clone());
        write_record(&root, &id, &record)?;
    }

    Ok(AddResult {
        id,
        already_stored: false,
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
    match value {
        Value::String(text) => Value::String(catalog::redact(text)),
        Value::Array(items) => Value::Array(items.iter().map(redact_json_strings).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), redact_json_strings(item)))
                .collect(),
        ),
        other => other.clone(),
    }
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
            "  version {} from {} added {} files {} bytes {}\n",
            catalog::redact(&version.hash),
            from,
            version.added_at,
            version.files,
            version.bytes
        ));
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
        for finding in &version.findings {
            let location = match finding.line {
                Some(line) => format!("{}:{}", catalog::redact(&finding.path), line),
                None => catalog::redact(&finding.path),
            };
            text.push_str(&format!(
                "    finding {} {} {}\n",
                severity_name(finding.severity),
                finding.rule,
                location
            ));
        }
    }
    text
}

/// Render the list of stored skills as text.
pub fn render_summaries(summaries: &[StoreSummary]) -> String {
    let mut text = String::new();
    for summary in summaries {
        text.push_str(&format!(
            "{}\t{}\thigh {}\tmedium {}\tlow {}\n",
            catalog::redact(&summary.id),
            catalog::redact(&summary.hash),
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
    fs::create_dir_all(&directory)
        .with_context(|| format!("create `{}`", catalog::redact_path(&directory)))?;
    let mut text = serde_json::to_string_pretty(record)?;
    text.push('\n');
    let path = record_path(root, id);
    let temporary = directory.join(format!("{id}.json.tmp-{}", std::process::id()));
    fs::write(&temporary, text)
        .with_context(|| format!("write record `{}`", catalog::redact_path(&temporary)))?;
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(anyhow::Error::new(error)
            .context(format!("write record `{}`", catalog::redact_path(&path))));
    }
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
        let written = std::io::copy(&mut entry, &mut out)
            .with_context(|| format!("read zip archive `{}`", catalog::redact_path(source)))?;
        written_total = written_total.saturating_add(written);
        if written_total > max_bytes {
            bail!("zip archive holds more than {max_bytes} bytes");
        }
    }
    Ok(())
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
        hasher.update(relative.as_bytes());
        hasher.update([0u8]);
        hasher.update(&content);
        hasher.update([0u8]);
    }
    let full = hex(&hasher.finalize());
    let hash = full[..HASH_LENGTH].to_string();
    Ok((hash, full, files.len() as u64, bytes))
}

/// The scripts and the findings of a skill root.
///
/// The scripts are found the same way the catalog finds them, so a skill
/// holds the same scripts in the store as in the index. The findings hold the
/// place of a secret, never its value.
fn scan_skill(root: &Path) -> Result<(Vec<StoredScript>, Vec<Finding>)> {
    let mut files = Vec::new();
    collect_files(root, "", &mut files)?;
    files.sort();

    let mut scripts = Vec::new();
    let mut findings = Vec::new();
    for relative in &files {
        let content = fs::read(root.join(relative))
            .with_context(|| format!("read `{}`", catalog::redact(relative)))?;
        if let Some(language) = catalog::script_language(Path::new(relative)) {
            let imports = if language == "python" {
                match String::from_utf8(content.clone()) {
                    Ok(text) => catalog::python_imports(&text)
                        .iter()
                        .map(|item| catalog::redact(item))
                        .collect(),
                    Err(_) => Vec::new(),
                }
            } else {
                Vec::new()
            };
            scripts.push(StoredScript {
                path: catalog::redact(relative),
                language: language.to_string(),
                imports,
            });
        }
        if is_binary(relative, &content) {
            findings.push(Finding {
                rule: "binary".to_string(),
                severity: Severity::Medium,
                path: catalog::redact(relative),
                line: None,
            });
        }
        // Every file is scanned, also a file that holds bytes that are not
        // text. The bytes are cut into lines and every line is read as text,
        // so a secret inside a binary payload is found as well.
        for (index, line) in content.split(|byte| *byte == b'\n').enumerate() {
            let line = String::from_utf8_lossy(line);
            if catalog::line_has_secret(&line) {
                findings.push(Finding {
                    rule: "secret".to_string(),
                    severity: Severity::High,
                    path: catalog::redact(relative),
                    line: Some(index as u64 + 1),
                });
            }
        }
    }
    Ok((scripts, findings))
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
            set_file_mode(&into, 0o444)?;
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
        set_directory_mode(&directory, 0o555)?;
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
/// same skill id never write the same record at the same time. The file goes
/// away with the lock, also when the add fails.
#[derive(Debug)]
struct Lock {
    path: PathBuf,
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
        let directory = root.join("locks");
        fs::create_dir_all(&directory)
            .with_context(|| format!("create `{}`", catalog::redact_path(&directory)))?;
        let path = directory.join(format!("{id}.lock"));
        let deadline = Instant::now() + timeout;
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
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

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
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
                Ok(()) => return Ok(Self { path }),
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
        set_directory_mode(&directory, 0o755)?;
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
        assert_eq!(secret.severity, Severity::High);

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
    fn the_stored_copy_is_read_only() {
        let source = skill_fixture("read-only");
        let store = temp_dir("read-only-store");

        let result = add_skill(&options(&source, &store)).unwrap();

        let stored = store.join("skills/demo").join(&result.version.hash);
        assert_eq!(mode_of(&stored.join("run.py")), 0o444);
        assert_eq!(mode_of(&stored.join("SKILL.md")), 0o444);
        assert_eq!(mode_of(&stored), 0o555);
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
        assert_eq!(finding.severity, Severity::Medium);
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
        assert_eq!(summaries[0].high, 1);
        assert_eq!(summaries[0].medium, 0);
        assert_eq!(summaries[0].low, 0);

        let text = render_summaries(&summaries);
        assert!(text.contains("demo"), "{text}");
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
        assert_eq!(finding.severity, Severity::High);
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
                }],
            }],
        };

        let text = redacted_record_json(&record).unwrap();
        assert!(!text.contains(SECRET), "{text}");
        assert!(text.contains("<redacted>"), "{text}");

        let rendered = render_record(&record);
        assert!(!rendered.contains(SECRET), "{rendered}");
    }
}
