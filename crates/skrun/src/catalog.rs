//! # skrun
//!
//! Catalog owns repository level bookkeeping for guidance skills.
//!
//! ## Owns
//! - skill discovery for folder and flat skills
//! - the parsing of the frontmatter of a skill
//! - deterministic JSON catalog index
//! - `SKILL_REGISTRY.md` reconciliation
//! - catalog consistency checks
//! - keyword search over the discovered skills
//! - the matching and the scoring that the search of the store shares
//! - redaction of every printed value that looks like a secret
//!
//! ## Must Not
//! - execute skills
//! - mutate skill payloads
//! - create a registry table that is not there
//! - call git or the network
//! - print a value that looks like a secret
//! - read the registry table out of a fenced code block
//!
//! ## Inputs
//! - skills directory
//! - optional registry and index files
//!
//! ## Outputs
//! - JSON catalog index
//! - registry drift report
//! - registry command exit status
//! - registry table error
//! - catalog check findings
//! - catalog search results
//!
//! ## Depends On
//! - regex
//! - serde_json
//!
//! ## Verify
//! - cargo test -p skrun

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

/// Schema version written into every generated catalog index.
pub const INDEX_SCHEMA_VERSION: u32 = 1;

/// Default registry file name inside a skills directory.
pub const REGISTRY_FILE: &str = "SKILL_REGISTRY.md";

/// Default committed index file name inside a skills directory.
pub const INDEX_FILE: &str = "skills.json";

/// Exact header row of the registry table.
pub const REGISTRY_HEADER: &str = "| Skill | When to read | Entrypoint |";

/// Exact separator row of the registry table.
pub const REGISTRY_SEPARATOR: &str = "| --- | --- | --- |";

/// Top level markdown files that never become flat skills.
const FLAT_EXCLUDED: [&str; 5] = [
    "SKILL_REGISTRY.md",
    "README.md",
    "README_CN.md",
    "AGENTS.md",
    "CLAUDE.md",
];

/// Maximum length of a generated registry summary.
const SUMMARY_LIMIT: usize = 160;

/// The marks that end the first sentence of a text.
///
/// Every mark holds the bytes that stay with the sentence: a full stop and a
/// Chinese full stop end the sentence before themselves, a question mark and
/// an exclamation mark stay with the sentence.
const SENTENCE_ENDS: [(&str, usize); 6] = [
    (". ", 0),
    ("。", 0),
    ("? ", 1),
    ("! ", 1),
    ("？", "？".len()),
    ("！", "！".len()),
];

/// Exact line that opens and closes a frontmatter block.
const FRONTMATTER_DELIMITER: &str = "---";

/// Placeholder that replaces user text that looks like a secret.
const REDACTED: &str = "<redacted>";

/// How a skill is stored inside the skills directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillKind {
    /// A directory with a `SKILL.md` entrypoint.
    Folder,
    /// A single markdown file at the top level of the skills directory.
    Flat,
}

/// Parsed frontmatter of a skill entrypoint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub tags: Vec<String>,
    pub requires: Vec<String>,
}

/// A skill discovered inside a skills directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub id: String,
    pub kind: SkillKind,
    /// Entrypoint path relative to the skills directory.
    pub entrypoint: PathBuf,
    /// Folder holding the skill, relative to the skills directory.
    pub folder: PathBuf,
    pub frontmatter: Frontmatter,
}

/// A script file that belongs to a folder skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptInfo {
    /// Script path relative to the skills directory.
    pub path: String,
    pub language: String,
    pub imports: Vec<String>,
}

/// A relative markdown link found in a skill entrypoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkInfo {
    /// Link target relative to the skills directory.
    pub target: String,
    pub exists: bool,
}

/// One skill entry of the catalog index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogSkill {
    pub id: String,
    pub kind: SkillKind,
    pub entrypoint: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub tags: Vec<String>,
    pub requires: Vec<String>,
    pub scripts: Vec<ScriptInfo>,
    pub links: Vec<LinkInfo>,
    pub bytes: u64,
}

/// The generated catalog index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogIndex {
    pub schema_version: u32,
    pub skills: Vec<CatalogSkill>,
}

/// A registry row that must be dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleRow {
    pub entrypoint: String,
    pub reason: &'static str,
}

/// A skill that has no registry row yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingRow {
    pub id: String,
    pub entrypoint: String,
}

/// Result of reconciling the registry table with the discovered skills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryReport {
    /// Registry file that was reconciled.
    pub registry: PathBuf,
    /// Registry file rendered relative to the skills directory.
    pub label: String,
    /// Rows that are dropped because their entrypoint is unusable.
    pub removed: Vec<StaleRow>,
    /// Rows dropped because the same entrypoint was already listed.
    pub duplicates: Vec<String>,
    /// Skills without a registry row.
    pub missing: Vec<MissingRow>,
    /// Entrypoints that keep their existing row.
    pub kept: Vec<String>,
    /// Whether the registry table was found at all.
    pub table_found: bool,
    /// Whether the registry file was rewritten.
    pub updated: bool,
}

impl RegistryReport {
    /// Whether the registry table no longer matches the discovered skills.
    pub fn out_of_sync(&self) -> bool {
        !self.table_found
            || !self.removed.is_empty()
            || !self.duplicates.is_empty()
            || !self.missing.is_empty()
    }

    /// Exit status of the `catalog registry` command for this report.
    ///
    /// A dry run reports drift as a failure so a stale table is noticed. A
    /// write run repaired the table already, so it succeeds; only a real
    /// error, such as a registry file that cannot be read or written, ends
    /// the command with a non-zero status.
    pub fn exit_code(&self, write: bool) -> i32 {
        if write {
            // The table was repaired, so the command succeeded.
            0
        } else if self.out_of_sync() {
            1
        } else {
            0
        }
    }

    /// Human readable summary of the drift.
    pub fn drift_summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.removed.is_empty() {
            parts.push(format!(
                "{} stale row{}",
                self.removed.len(),
                plural(self.removed.len())
            ));
        }
        if !self.duplicates.is_empty() {
            parts.push(format!(
                "{} duplicate row{}",
                self.duplicates.len(),
                plural(self.duplicates.len())
            ));
        }
        if !self.missing.is_empty() {
            parts.push(format!(
                "{} missing row{}",
                self.missing.len(),
                plural(self.missing.len())
            ));
        }
        if parts.is_empty() {
            return "registry table is missing".to_string();
        }
        parts.join(", ")
    }

    /// Render the report as deterministic text.
    ///
    /// Every user controlled text goes through the redaction helper, so a
    /// value that looks like a secret never reaches the output.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("registry: {}\n", redact(&self.label)));
        out.push_str(&format!(
            "status: {}\n",
            if self.out_of_sync() {
                "out of sync"
            } else {
                "in sync"
            }
        ));
        for row in &self.removed {
            out.push_str(&format!(
                "stale row ({}): {}\n",
                row.reason,
                redact(&row.entrypoint)
            ));
        }
        for entrypoint in &self.duplicates {
            out.push_str(&format!("duplicate row: {}\n", redact(entrypoint)));
        }
        for row in &self.missing {
            out.push_str(&format!(
                "missing row: {} ({})\n",
                redact(&row.id),
                redact(&row.entrypoint)
            ));
        }
        if self.updated {
            out.push_str(&format!("updated registry: {}\n", redact(&self.label)));
        }
        out
    }
}

/// Severity of a catalog finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One catalog finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
}

impl Finding {
    fn error(message: String) -> Self {
        Self {
            severity: Severity::Error,
            message,
        }
    }

    fn warning(message: String) -> Self {
        Self {
            severity: Severity::Warning,
            message,
        }
    }
}

/// Result of checking a skills directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub findings: Vec<Finding>,
    pub skill_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
}

impl CheckReport {
    /// Whether the check found at least one error.
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    /// Render the findings and the summary as deterministic text.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for finding in &self.findings {
            let prefix = match finding.severity {
                Severity::Error => "error: ",
                Severity::Warning => "warning: ",
            };
            out.push_str(prefix);
            out.push_str(&finding.message);
            out.push('\n');
        }
        out.push_str(&format!(
            "checked {} skill{}: {} error{}, {} warning{}\n",
            self.skill_count,
            plural(self.skill_count),
            self.error_count,
            plural(self.error_count),
            self.warning_count,
            plural(self.warning_count)
        ));
        out
    }
}

/// Discover every skill inside `dir`.
///
/// Folder skills are immediate subdirectories with a `SKILL.md` file, flat
/// skills are top level markdown files. The result is sorted by skill id.
pub fn discover_skills(dir: &Path) -> Result<Vec<Skill>> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("read skills directory `{}`", redact_path(dir)))?;
    let mut skills = Vec::new();

    for entry in entries {
        let entry =
            entry.with_context(|| format!("read skills directory `{}`", redact_path(dir)))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };

        if metadata.is_dir() {
            if name.starts_with('.') {
                continue;
            }
            let entrypoint = path.join("SKILL.md");
            if !entrypoint.is_file() {
                continue;
            }
            skills.push(Skill {
                id: name.clone(),
                kind: SkillKind::Folder,
                entrypoint: PathBuf::from(&name).join("SKILL.md"),
                folder: PathBuf::from(&name),
                frontmatter: read_frontmatter(&entrypoint),
            });
        } else if metadata.is_file() && is_flat_skill_file(&name) {
            skills.push(Skill {
                id: file_stem(&name),
                kind: SkillKind::Flat,
                entrypoint: PathBuf::from(&name),
                folder: PathBuf::new(),
                frontmatter: read_frontmatter(&path),
            });
        }
    }

    skills.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.entrypoint.cmp(&right.entrypoint))
    });
    Ok(skills)
}

/// Build the catalog index for `dir`.
pub fn build_index(dir: &Path) -> Result<CatalogIndex> {
    let skills = discover_skills(dir)?;
    let mut entries = Vec::new();

    for skill in &skills {
        let path = dir.join(&skill.entrypoint);
        let text = fs::read_to_string(&path).unwrap_or_default();
        let bytes = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        let scripts = match skill.kind {
            SkillKind::Folder => collect_scripts(dir, skill)?,
            SkillKind::Flat => Vec::new(),
        };
        let links = collect_links(dir, skill, &text);
        entries.push(CatalogSkill {
            id: skill.id.clone(),
            kind: skill.kind,
            entrypoint: path_to_string(&skill.entrypoint),
            name: skill.frontmatter.name.clone(),
            description: skill.frontmatter.description.clone(),
            version: skill.frontmatter.version.clone(),
            tags: skill.frontmatter.tags.clone(),
            requires: skill.frontmatter.requires.clone(),
            scripts,
            links,
            bytes,
        });
    }

    Ok(CatalogIndex {
        schema_version: INDEX_SCHEMA_VERSION,
        skills: entries,
    })
}

/// Render the catalog index as pretty JSON with a trailing newline.
///
/// Every string of the index goes through the redaction helper, so a value
/// that looks like a secret never reaches the output. A value that holds no
/// secret stays as it is, so the paths of the index stay real.
pub fn render_index(index: &CatalogIndex) -> Result<String> {
    let mut text = serde_json::to_string_pretty(&redacted_index(index))?;
    text.push('\n');
    Ok(text)
}

/// The catalog index with every string redacted.
///
/// Every string of the index goes through the redaction helper, so a value
/// that looks like a secret never reaches the output. A value that holds no
/// secret stays as it is, so the paths of the index stay real.
fn redacted_index(index: &CatalogIndex) -> CatalogIndex {
    CatalogIndex {
        schema_version: index.schema_version,
        skills: index.skills.iter().map(redacted_skill).collect(),
    }
}

/// One skill of the index with every string redacted.
fn redacted_skill(skill: &CatalogSkill) -> CatalogSkill {
    CatalogSkill {
        id: redact(&skill.id),
        kind: skill.kind,
        entrypoint: redact(&skill.entrypoint),
        name: skill.name.as_deref().map(redact),
        description: skill.description.as_deref().map(redact),
        version: skill.version.as_deref().map(redact),
        tags: skill.tags.iter().map(|tag| redact(tag)).collect(),
        requires: skill.requires.iter().map(|name| redact(name)).collect(),
        scripts: skill.scripts.iter().map(redacted_script).collect(),
        links: skill.links.iter().map(redacted_link).collect(),
        bytes: skill.bytes,
    }
}

/// One script of the index with every string redacted.
fn redacted_script(script: &ScriptInfo) -> ScriptInfo {
    ScriptInfo {
        path: redact(&script.path),
        language: redact(&script.language),
        imports: script.imports.iter().map(|item| redact(item)).collect(),
    }
}

/// One link of the index with every string redacted.
fn redacted_link(link: &LinkInfo) -> LinkInfo {
    LinkInfo {
        target: redact(&link.target),
        exists: link.exists,
    }
}

/// Redact every string value of a JSON value.
///
/// Every string of the value goes through the redaction helper of the
/// crate, so a value that looks like a secret never reaches the output.
/// The keys of an object are the names of the fields of the value, so they
/// hold no user text.
pub fn redact_json_strings(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact(text)),
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

/// Write rendered index text to `path`, creating parent folders when needed.
pub fn write_index_file(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("create directory `{}`", redact_path(parent)))?;
    }
    fs::write(path, text).with_context(|| format!("write index `{}`", redact_path(path)))
}

/// Reconcile the registry table with the discovered skills.
///
/// Without `write` the registry file is never touched and only the drift is
/// reported. With `write` stale and duplicate rows are dropped and rows for
/// missing skills are appended; everything outside the table is preserved.
///
/// A registry file without the registry table is an error: the rows have no
/// place to go, so the file is left exactly as it is.
pub fn reconcile_registry(dir: &Path, registry: &Path, write: bool) -> Result<RegistryReport> {
    let skills = discover_skills(dir)?;
    let content = read_registry(registry)?;
    let document = RegistryDocument::parse(&content);
    reconcile_document(dir, registry, &document, &skills, write)
}

/// Reconcile the parsed registry document with the discovered skills.
///
/// The registry table has to be there; a document without it is an error and
/// the registry file is never written.
fn reconcile_document(
    dir: &Path,
    registry: &Path,
    document: &RegistryDocument,
    skills: &[Skill],
    write: bool,
) -> Result<RegistryReport> {
    let Some((header, start, end)) = document.table() else {
        bail!(
            "registry `{}` has no `{}` table",
            redact_path(registry),
            REGISTRY_HEADER
        );
    };

    let mut by_entrypoint: BTreeMap<String, &Skill> = BTreeMap::new();
    for skill in skills {
        by_entrypoint.insert(path_to_string(&skill.entrypoint), skill);
    }

    let mut removed = Vec::new();
    let mut duplicates = Vec::new();
    let mut kept = Vec::new();
    let mut kept_rows: Vec<Line> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for line in &document.lines[start..end] {
        let cells = parse_cells(&line.text);
        let Some(cell) = cells.get(2) else {
            removed.push(StaleRow {
                entrypoint: line.text.trim().to_string(),
                reason: "missing entrypoint cell",
            });
            continue;
        };
        let Some(raw) = extract_link_path(cell) else {
            removed.push(StaleRow {
                entrypoint: cell.clone(),
                reason: "invalid entrypoint cell",
            });
            continue;
        };
        let entrypoint = path_to_string(&normalize_relative(Path::new(&raw)));
        if !by_entrypoint.contains_key(&entrypoint) {
            let reason = if dir.join(&entrypoint).exists() {
                "not a skill"
            } else {
                "file missing"
            };
            removed.push(StaleRow { entrypoint, reason });
            continue;
        }
        if !seen.insert(entrypoint.clone()) {
            duplicates.push(entrypoint);
            continue;
        }
        kept.push(entrypoint);
        kept_rows.push(line.clone());
    }

    let missing: Vec<MissingRow> = skills
        .iter()
        .filter(|skill| !seen.contains(&path_to_string(&skill.entrypoint)))
        .map(|skill| MissingRow {
            id: skill.id.clone(),
            entrypoint: path_to_string(&skill.entrypoint),
        })
        .collect();

    let updated = write && (!removed.is_empty() || !duplicates.is_empty() || !missing.is_empty());
    if updated {
        let rows: Vec<String> = missing
            .iter()
            .filter_map(|row| {
                skills
                    .iter()
                    .find(|skill| path_to_string(&skill.entrypoint) == row.entrypoint)
                    .map(registry_row)
            })
            .collect();
        write_registry(document, (header, start, end), &kept_rows, &rows, registry)?;
    }

    Ok(RegistryReport {
        registry: registry.to_path_buf(),
        label: redact(&display_path(dir, registry)),
        removed,
        duplicates,
        missing,
        kept,
        table_found: document.header.is_some(),
        updated,
    })
}

/// Check a skills directory for catalog problems.
pub fn check_catalog(
    dir: &Path,
    registry: Option<&Path>,
    index: Option<&Path>,
) -> Result<CheckReport> {
    let skills = discover_skills(dir)?;
    let catalog = build_index(dir)?;
    let mut findings = Vec::new();

    for skill in &skills {
        if skill.kind != SkillKind::Folder {
            continue;
        }
        match &skill.frontmatter.name {
            None => findings.push(Finding::error(format!(
                "skill `{}` is missing frontmatter `name`",
                redact(&skill.id)
            ))),
            Some(name) if name != &skill.id => findings.push(Finding::warning(format!(
                "skill `{}` has frontmatter name `{}` that differs from its id",
                redact(&skill.id),
                redact(name)
            ))),
            Some(_) => {}
        }
        if skill.frontmatter.description.is_none() {
            findings.push(Finding::error(format!(
                "skill `{}` is missing frontmatter `description`",
                redact(&skill.id)
            )));
        }
    }

    for skill in &catalog.skills {
        for link in &skill.links {
            if !link.exists {
                findings.push(Finding::error(format!(
                    "skill `{}` has a broken link `{}`",
                    redact(&skill.id),
                    redact(&link.target)
                )));
            }
        }
    }

    let registry_path = match registry {
        Some(path) => Some(path.to_path_buf()),
        None => {
            let candidate = dir.join(REGISTRY_FILE);
            if candidate.exists() {
                Some(candidate)
            } else {
                None
            }
        }
    };
    if let Some(path) = registry_path {
        let label = redact(&display_path(dir, &path));
        if path.exists() {
            let content = read_registry(&path)?;
            let document = RegistryDocument::parse(&content);
            if document.table().is_none() {
                findings.push(Finding::error(format!(
                    "registry `{label}` has no `{REGISTRY_HEADER}` table"
                )));
            } else {
                let report = reconcile_document(dir, &path, &document, &skills, false)?;
                if report.out_of_sync() {
                    findings.push(Finding::error(format!(
                        "registry `{label}` is out of sync ({})",
                        report.drift_summary()
                    )));
                }
            }
        } else {
            findings.push(Finding::error(format!("registry `{label}` is missing")));
        }
    }

    for (path, line) in scan_secrets(dir)? {
        // The path goes through the same redaction helper as every other
        // printed value, so a secret shaped file or directory name is hidden.
        findings.push(Finding::error(format!(
            "possible secret in `{}:{}`",
            redact(&path),
            line
        )));
    }

    let index_path = match index {
        Some(path) => Some(path.to_path_buf()),
        None => {
            let candidate = dir.join(INDEX_FILE);
            if candidate.exists() {
                Some(candidate)
            } else {
                None
            }
        }
    };
    if let Some(path) = index_path
        && index_is_stale(&path, &catalog)
    {
        findings.push(Finding::error(
            "index is stale; run skrun catalog index".to_string(),
        ));
    }

    let error_count = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .count();
    let warning_count = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Warning)
        .count();

    Ok(CheckReport {
        findings,
        skill_count: skills.len(),
        error_count,
        warning_count,
    })
}

/// Default number of search results.
pub const SEARCH_LIMIT: usize = 10;

/// Maximum length of the description sentence that a search result shows.
pub const SEARCH_SUMMARY_LIMIT: usize = 120;

/// The weight of the skill id in a search score.
pub const SEARCH_WEIGHT_ID: u32 = 5;

/// The weight of the frontmatter name in a search score.
pub const SEARCH_WEIGHT_NAME: u32 = 4;

/// The weight of the registry title in a search score.
const SEARCH_WEIGHT_TITLE: u32 = 4;

/// The weight of the registry summary in a search score.
const SEARCH_WEIGHT_SUMMARY: u32 = 3;

/// The weight of the frontmatter description in a search score.
pub const SEARCH_WEIGHT_DESCRIPTION: u32 = 3;

/// The weight of a frontmatter tag in a search score.
pub const SEARCH_WEIGHT_TAG: u32 = 3;

/// The weight of the entrypoint body in a search score.
pub const SEARCH_WEIGHT_BODY: u32 = 1;

/// Options of a catalog search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// The raw query; every whitespace separated term has to match.
    pub query: String,
    /// The kind of skill to keep; every kind when it is `None`.
    pub kind: Option<SkillKind>,
    /// Whether only the skills with at least one script are kept.
    pub has_scripts: bool,
    /// The number of results to keep.
    pub limit: usize,
}

impl SearchOptions {
    /// The options of a search for `query` without any filter.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            kind: None,
            has_scripts: false,
            limit: SEARCH_LIMIT,
        }
    }
}

/// One skill that matches a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    /// The id of the skill.
    pub id: String,
    /// How the skill is stored inside the skills directory.
    pub kind: SkillKind,
    /// The entrypoint path relative to the skills directory.
    pub entrypoint: String,
    /// The score of the match; a higher score is a better match.
    pub score: u32,
    /// The title of the skill; the registry title, else the frontmatter name.
    pub title: Option<String>,
    /// The summary of the skill; the registry summary, else the first
    /// sentence of the description. A summary that holds a secret is
    /// redacted, so a secret is never printed in part.
    pub summary: Option<String>,
}

/// Search the skills of `dir` for the terms of `options`.
///
/// Every term of the query has to occur in at least one field of a skill, so
/// the terms narrow the result down together. A term that occurs in several
/// fields adds the weight of every field, so a skill that matches on its id
/// outranks a skill that only mentions the term in its body.
///
/// The result is sorted by score and then by id, so the same directory always
/// gives the same result.
pub fn search_skills(dir: &Path, options: &SearchOptions) -> Result<Vec<SearchHit>> {
    let terms = search_terms(&options.query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let skills = discover_skills(dir)?;
    let rows = read_registry_rows(dir)?;

    let mut hits = Vec::new();
    for skill in &skills {
        if let Some(kind) = options.kind
            && skill.kind != kind
        {
            continue;
        }
        if options.has_scripts && !skill_has_scripts(dir, skill)? {
            continue;
        }

        let entrypoint = path_to_string(&skill.entrypoint);
        let row = rows.get(&entrypoint);
        let fields = search_fields(dir, skill, row);
        let Some(score) = score_fields(&fields, &terms) else {
            continue;
        };

        let title = row
            .map(|row| row.title.clone())
            .filter(|title| !title.is_empty())
            .or_else(|| skill.frontmatter.name.clone());
        // The summary goes through the redaction helper before it is cut and
        // truncated, so a secret that crosses the cut is never printed in
        // part.
        let summary = row
            .map(|row| row.summary.clone())
            .filter(|summary| !summary.is_empty())
            .or_else(|| {
                skill
                    .frontmatter
                    .description
                    .as_deref()
                    .map(|description| first_sentence(description, SEARCH_SUMMARY_LIMIT))
                    .filter(|sentence| !sentence.is_empty())
            })
            .map(|summary| redact(&summary));

        hits.push(SearchHit {
            id: skill.id.clone(),
            kind: skill.kind,
            entrypoint,
            score,
            title,
            summary,
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

/// Render the search hits as text.
///
/// Every printed value goes through the redaction helper, so a value that
/// looks like a secret never reaches the output.
pub fn render_search_text(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "no skills match\n".to_string();
    }

    let mut out = String::new();
    for hit in hits {
        let label = hit.title.clone().unwrap_or_else(|| hit.id.clone());
        out.push_str(&format!(
            "{}  {}  {}  {}\n",
            redact(&hit.id),
            kind_name(hit.kind),
            redact(&hit.entrypoint),
            redact(&label)
        ));
        if let Some(summary) = &hit.summary {
            out.push_str(&format!("  {}\n", redact(summary)));
        }
    }
    out
}

/// Render the search hits as JSON.
///
/// The values go through the same redaction helper as the text output.
pub fn render_search_json(hits: &[SearchHit]) -> Result<String> {
    let redacted: Vec<SearchHit> = hits.iter().map(redacted_hit).collect();
    let mut text = serde_json::to_string_pretty(&redacted)?;
    text.push('\n');
    Ok(text)
}

/// The terms of a search query.
///
/// The query is split on whitespace and every term is lowercased, so the
/// match is case insensitive. The lowercasing is Unicode aware, so a Chinese
/// term matches like an English one and no word segmentation is needed.
///
/// The search of the store uses the same terms, so a query means the same
/// thing in both searches.
pub fn search_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|term| term.to_lowercase())
        .filter(|term| !term.is_empty())
        .collect()
}

/// The fields of a skill that a search term can match.
///
/// Every field is lowercased once, so the terms only have to be lowercased
/// once as well.
fn search_fields(dir: &Path, skill: &Skill, row: Option<&RegistryRow>) -> Vec<(String, u32)> {
    let mut fields: Vec<(String, u32)> = Vec::new();
    fields.push((skill.id.to_lowercase(), SEARCH_WEIGHT_ID));
    if let Some(name) = &skill.frontmatter.name {
        fields.push((name.to_lowercase(), SEARCH_WEIGHT_NAME));
    }
    if let Some(row) = row {
        fields.push((row.title.to_lowercase(), SEARCH_WEIGHT_TITLE));
        fields.push((row.summary.to_lowercase(), SEARCH_WEIGHT_SUMMARY));
    }
    if let Some(description) = &skill.frontmatter.description {
        fields.push((description.to_lowercase(), SEARCH_WEIGHT_DESCRIPTION));
    }
    for tag in &skill.frontmatter.tags {
        fields.push((tag.to_lowercase(), SEARCH_WEIGHT_TAG));
    }
    let body = fs::read_to_string(dir.join(&skill.entrypoint)).unwrap_or_default();
    fields.push((body.to_lowercase(), SEARCH_WEIGHT_BODY));
    fields
}

/// The score of a skill, when every term matches one of its fields.
///
/// The score is the sum of the weight of every field that holds a term, so a
/// term that occurs in the id and in the title counts for both.
///
/// The search of the store uses the same scoring, so a skill of the store
/// and a skill of a directory are ranked by the same rules.
pub fn score_fields(fields: &[(String, u32)], terms: &[String]) -> Option<u32> {
    let mut score = 0;
    for term in terms {
        let mut matched = false;
        for (text, weight) in fields {
            if text.contains(term.as_str()) {
                score += weight;
                matched = true;
            }
        }
        if !matched {
            return None;
        }
    }
    Some(score)
}

/// A row of the registry table.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistryRow {
    /// The title of the skill; the first cell of the row.
    title: String,
    /// The summary of the skill; the second cell of the row.
    summary: String,
}

/// The rows of the registry table of `dir`, keyed by their entrypoint.
///
/// The rows are only read when the registry file exists and holds the
/// registry table; a missing file or a file without the table gives no row.
fn read_registry_rows(dir: &Path) -> Result<BTreeMap<String, RegistryRow>> {
    let mut rows = BTreeMap::new();
    let registry = dir.join(REGISTRY_FILE);
    if !registry.exists() {
        return Ok(rows);
    }
    let content = read_registry(&registry)?;
    let document = RegistryDocument::parse(&content);
    let Some((_, start, end)) = document.table() else {
        return Ok(rows);
    };

    for line in &document.lines[start..end] {
        let cells = parse_cells(&line.text);
        let (Some(title), Some(summary), Some(entrypoint)) =
            (cells.first(), cells.get(1), cells.get(2))
        else {
            continue;
        };
        let Some(raw) = extract_link_path(entrypoint) else {
            continue;
        };
        let key = path_to_string(&normalize_relative(Path::new(&raw)));
        rows.entry(key).or_insert_with(|| RegistryRow {
            title: cell_label(title),
            summary: summary.clone(),
        });
    }
    Ok(rows)
}

/// The label of a registry cell.
///
/// A cell that holds a markdown link shows the label of the link, so the
/// title of a row reads as the name of the skill.
fn cell_label(cell: &str) -> String {
    let trimmed = cell.trim();
    if let Some(end) = trimmed.find("](")
        && trimmed.starts_with('[')
    {
        return trimmed[1..end].trim().to_string();
    }
    trimmed.to_string()
}

/// The first sentence of a text, at most `limit` characters long.
///
/// The text goes through the redaction helper before the sentence is cut out,
/// so a secret that crosses the cut is never printed in part.
///
/// The sentence ends at the first full stop, at the first question mark, at
/// the first exclamation mark, at the first Chinese full stop, at the first
/// Chinese question mark, at the first Chinese exclamation mark or at the
/// first line break; the earliest of them ends the sentence.
///
/// The search of the store uses the same summary, so a skill of the store
/// and a skill of a directory show the same text.
pub fn first_sentence(text: &str, limit: usize) -> String {
    let redacted = redact(text);
    let mut cut = redacted.len();
    for (mark, keep) in SENTENCE_ENDS {
        if let Some(position) = redacted.find(mark) {
            cut = cut.min(position + keep);
        }
    }
    if let Some(position) = redacted.find('\n') {
        cut = cut.min(position);
    }
    let sentence = redacted.get(..cut).unwrap_or(redacted.as_str()).trim();
    if sentence.chars().count() > limit {
        sentence.chars().take(limit).collect()
    } else {
        sentence.to_string()
    }
}

/// Whether a skill holds at least one script.
///
/// The scripts are found the same way the catalog index finds them, so a
/// skill with a script is the same skill in both commands.
fn skill_has_scripts(dir: &Path, skill: &Skill) -> Result<bool> {
    match skill.kind {
        SkillKind::Folder => Ok(!collect_scripts(dir, skill)?.is_empty()),
        SkillKind::Flat => Ok(false),
    }
}

/// A search hit with every value that goes through the redaction helper.
fn redacted_hit(hit: &SearchHit) -> SearchHit {
    SearchHit {
        id: redact(&hit.id),
        kind: hit.kind,
        entrypoint: redact(&hit.entrypoint),
        score: hit.score,
        title: hit.title.as_deref().map(redact),
        summary: hit.summary.as_deref().map(redact),
    }
}

/// The name of a skill kind, as it is printed.
fn kind_name(kind: SkillKind) -> &'static str {
    match kind {
        SkillKind::Folder => "folder",
        SkillKind::Flat => "flat",
    }
}

/// Parse the optional frontmatter block of a skill entrypoint.
///
/// The block only counts when its first line is exactly `---` and a later
/// line is exactly `---` as well. An unterminated block is not frontmatter.
///
/// The values of the block are unquoted, so a name that is wrapped in
/// quotes is the name without its quotes.
pub fn parse_frontmatter(text: &str) -> Frontmatter {
    let mut frontmatter = Frontmatter::default();
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return frontmatter;
    };
    if first != FRONTMATTER_DELIMITER {
        return frontmatter;
    }

    let mut closed = false;
    for line in lines {
        if line == FRONTMATTER_DELIMITER {
            closed = true;
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "name" => frontmatter.name = non_empty(&unquote_scalar(value)),
            "description" => frontmatter.description = non_empty(&unquote_scalar(value)),
            "version" => frontmatter.version = non_empty(&unquote_scalar(value)),
            "tags" => frontmatter.tags = parse_list(value),
            "requires" => frontmatter.requires = parse_list(value),
            _ => {}
        }
    }

    if closed {
        frontmatter
    } else {
        Frontmatter::default()
    }
}

fn read_frontmatter(path: &Path) -> Frontmatter {
    let text = fs::read_to_string(path).unwrap_or_default();
    parse_frontmatter(&text)
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Parse a frontmatter list value.
///
/// A list is written as `[a, b]` or as `a, b`. A comma that is inside a
/// quoted item does not end the item, so `["one, two", three]` holds two
/// items and not three.
///
/// Every item is unquoted, so a quoted item loses its quotes.
fn parse_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let inner = if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    split_list_items(inner)
        .iter()
        .map(|item| unquote_scalar(item))
        .filter(|item| !item.is_empty())
        .collect()
}

/// Split a list value into its items.
///
/// A comma ends an item, unless the comma is inside a quoted item. An item
/// that starts with a double quote is a double quoted item and ends at its
/// closing double quote, an item that starts with a single quote is a single
/// quoted item and ends at its closing single quote.
///
/// Two single quotes inside a single quoted item are one quote that stays
/// inside the item, and a backslash inside a double quoted item keeps the
/// next character inside the item.
fn split_list_items(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut item = String::new();
    let mut quote: Option<char> = None;
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        match quote {
            Some('"') => {
                item.push(character);
                if character == '\\' {
                    if let Some(escaped) = chars.next() {
                        item.push(escaped);
                    }
                    continue;
                }
                if character == '"' {
                    quote = None;
                }
            }
            Some('\'') => {
                item.push(character);
                if character == '\'' {
                    if chars.peek() == Some(&'\'') {
                        item.push(chars.next().unwrap_or('\''));
                        continue;
                    }
                    quote = None;
                }
            }
            _ => match character {
                '"' | '\'' if item.trim().is_empty() => {
                    quote = Some(character);
                    item.push(character);
                }
                ',' => items.push(std::mem::take(&mut item)),
                _ => item.push(character),
            },
        }
    }
    items.push(item);
    items
}

/// Unquote a frontmatter scalar.
///
/// A value that is wrapped in matching double quotes or in matching single
/// quotes loses its quotes. A double quoted value also unescapes `\"` and
/// `\\`, a single quoted value turns `''` into a `'`.
///
/// A value that is not wrapped in matching quotes keeps its quotes, so a
/// value that only starts with a quote is left as it is.
pub fn unquote_scalar(value: &str) -> String {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if first == b'"' && last == b'"' {
            return unescape_double_quoted(&trimmed[1..trimmed.len() - 1]);
        }
        if first == b'\'' && last == b'\'' {
            return trimmed[1..trimmed.len() - 1].replace("''", "'");
        }
    }
    trimmed.to_string()
}

/// Unescape the escapes of a double quoted scalar.
///
/// `\"` becomes a `"` and `\\` becomes a `\\`, every other escape is kept
/// as it is.
fn unescape_double_quoted(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            text.push(character);
            continue;
        }
        match chars.next() {
            Some('"') => text.push('"'),
            Some('\\') => text.push('\\'),
            Some(other) => {
                text.push('\\');
                text.push(other);
            }
            None => text.push('\\'),
        }
    }
    text
}

fn is_flat_skill_file(name: &str) -> bool {
    if FLAT_EXCLUDED.contains(&name) {
        return false;
    }
    name.ends_with(".md")
}

fn file_stem(name: &str) -> String {
    match name.rfind('.') {
        Some(position) if position > 0 => name[..position].to_string(),
        _ => name.to_string(),
    }
}

fn collect_scripts(dir: &Path, skill: &Skill) -> Result<Vec<ScriptInfo>> {
    let root = dir.join(&skill.folder);
    let mut files = Vec::new();
    if root.is_dir() {
        walk_script_files(&root, &mut files)?;
    }

    let mut scripts = Vec::new();
    for file in files {
        let Some(language) = script_language(&file) else {
            continue;
        };
        let imports = if language == "python" {
            let text = fs::read_to_string(&file).unwrap_or_default();
            python_imports(&text)
        } else {
            Vec::new()
        };
        scripts.push(ScriptInfo {
            path: path_to_string(&relative_to(dir, &file)),
            language: language.to_string(),
            imports,
        });
    }
    scripts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(scripts)
}

fn walk_script_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(root)
        .with_context(|| format!("read skill directory `{}`", redact_path(root)))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("read skill directory `{}`", redact_path(root)))?;
        paths.push(entry.path());
    }
    paths.sort();

    for path in paths {
        let file_type = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata.file_type(),
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            if let Ok(target) = fs::metadata(&path)
                && target.is_file()
            {
                out.push(path);
            }
            continue;
        }
        if file_type.is_dir() {
            let name = file_name_of(&path);
            if name.starts_with('.') || name == "__pycache__" {
                continue;
            }
            walk_script_files(&path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

pub(crate) fn script_language(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("py") | Some("pyw") => Some("python"),
        Some("sh") => Some("shell"),
        Some("js") | Some("mjs") => Some("javascript"),
        Some("ts") => Some("typescript"),
        _ => None,
    }
}

/// Whether a file holds python code.
///
/// A file with the extension of Python holds python code, and a file whose
/// first line names a python interpreter holds python code as well, so the
/// shebang of a file of the shell names the language of the file.
pub(crate) fn is_python_file(path: &Path, text: &str) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("py") | Some("pyw")
    ) || shebang_language(text) == Some("python")
}

/// The language of a script that names its interpreter in its first line.
///
/// The interpreter of the line names the language of the script. A shell
/// interpreter is a shell script, a python interpreter is a python script
/// and a javascript interpreter is a javascript script. Every other
/// interpreter is a script of an unknown language.
///
/// A file that starts with no `#!` line names no interpreter, so it holds
/// no language of its own.
pub fn shebang_language(text: &str) -> Option<&'static str> {
    let first = text.split('\n').next()?;
    let rest = first.strip_prefix("#!")?;
    let mut words = rest.split_whitespace();
    let mut interpreter = words.next()?;
    if Path::new(interpreter)
        .file_name()
        .and_then(|name| name.to_str())
        == Some("env")
    {
        interpreter = env_interpreter(&mut words)?;
    }
    let name = Path::new(interpreter)
        .file_name()
        .and_then(|name| name.to_str())?
        .to_ascii_lowercase();
    Some(match name.as_str() {
        "sh" | "bash" | "zsh" | "dash" => "shell",
        name if name.starts_with("python") => "python",
        "node" | "deno" | "bun" => "javascript",
        _ => "other",
    })
}

/// The interpreter of a shebang that names `env`.
///
/// The options of `env` come before the interpreter, so the options and
/// the names of the environment are skipped: the interpreter of
/// `#!/usr/bin/env python3` is `python3` and the interpreter of
/// `#!/usr/bin/env -S python3 -u` is `python3`.
///
/// The option `-S` splits the rest of the line into words, so the
/// interpreter is the first word of the string behind it: the interpreter
/// of `#!/usr/bin/env -S "python3 -u"` is `python3`.
fn env_interpreter<'a>(words: &mut impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let all: Vec<&'a str> = words.collect();
    let mut index = 0;
    while index < all.len() {
        let word = all[index];
        index += 1;
        if word == "--" {
            // The word behind the `--` is the interpreter, for no option
            // and no name of the environment stands in front of it.
            return all.get(index).map(|word| word.trim_matches('"'));
        }
        if let Some(rest) = split_string(word) {
            // The option `-S` splits the rest of the line into words, so
            // the words behind it are the words of the interpreter.
            let mut split: Vec<&str> = rest.split_whitespace().collect();
            if split.is_empty() {
                // The string of the option `-S` is the word behind it.
                split.extend(all[index.min(all.len())..].iter().copied());
            }
            return split
                .into_iter()
                .map(|word| word.trim_matches('"'))
                .find(|word| {
                    !word.is_empty() && !word.starts_with('-') && !is_environment_name(word)
                });
        }
        if takes_a_name(word) {
            // The option takes a name, so the word behind it names no
            // interpreter.
            index += 1;
            continue;
        }
        if word.starts_with('-') {
            // An option of env, so the word names no interpreter.
            continue;
        }
        if is_environment_name(word) {
            // A name of the environment, so the word names no interpreter.
            continue;
        }
        return Some(word.trim_matches('"'));
    }
    None
}

/// Whether an option of `env` takes a name.
///
/// The option `-u` takes the name of a variable of the environment and the
/// option `-C` takes the name of a folder, so the word behind such an
/// option names no interpreter.
fn takes_a_name(word: &str) -> bool {
    matches!(word, "-u" | "--unset" | "-C" | "--chdir")
}

/// The string of the option `-S` of `env`.
///
/// The option `-S` takes the rest of the line as a string, so the string
/// is what stands behind the option: the string of `-S python3` is empty
/// and the string of `-Spython3` is `python3`.
fn split_string(word: &str) -> Option<&str> {
    if let Some(rest) = word.strip_prefix("--split-string=") {
        return Some(rest);
    }
    let rest = word.strip_prefix("-S")?;
    Some(rest)
}

/// Whether a word of a shebang names a variable of the environment.
///
/// A name of the environment is a name and a value, so `LANG=C` names no
/// interpreter.
fn is_environment_name(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// The modules that a python text imports.
///
/// Every module of an import statement is read, so `import a, b as c, d.e`
/// holds three modules: `a`, `b` and `d`. The name of a module is the top
/// level name of its path, so `d.e` is the module `d` and `from x.y import
/// z` is the module `x`.
///
/// A statement that goes on the next line is read as one line, so every
/// line of the text is read.
///
/// A line that is inside a string is a part of the string, so a line of a
/// docstring holds no import, not even when it looks like one.
///
/// A relative import names a module of the skill itself, so it is left out.
///
/// An import that follows a colon on the same line is an import as well,
/// so `if True: import subprocess` holds the module `subprocess` and
/// `try: import socket` holds the module `socket`.
///
/// A call of `__import__` with a string and a call of
/// `importlib.import_module` with a string import a module as well, so
/// `__import__("socket")` holds the module `socket`.
pub fn python_imports(text: &str) -> Vec<String> {
    let mut imports = BTreeSet::new();
    for line in logical_lines(text) {
        read_line_imports(&line, &mut imports);
    }
    for module in dynamic_imports(text) {
        imports.insert(module);
    }
    imports
        .into_iter()
        .filter(|name| name != "__future__")
        .collect()
}

/// Whether a python text can be tokenised.
///
/// A python text that holds a string that is never closed, a bracket that
/// is never closed or an indentation that the interpreter refuses cannot be
/// tokenised, so the imports of the text cannot be read with certainty.
pub fn python_parses(text: &str) -> bool {
    if !printable_only(text) {
        return false;
    }
    let Some(mask) = python_code_mask(text) else {
        return false;
    };
    indentation_fits(text, &mask)
}

/// Whether a python text holds only characters that the tokenizer accepts.
///
/// The tokenizer of the interpreter refuses a text that holds a null byte
/// or another character that cannot be printed, so such a text cannot be
/// tokenised.
fn printable_only(text: &str) -> bool {
    text.bytes()
        .all(|byte| matches!(byte, b'\t' | b'\n' | b'\r' | 0x0c) || (byte >= 0x20 && byte != 0x7f))
}

/// Whether the lines of a python text fit together.
///
/// The interpreter reads the indentation of the lines: a line that is
/// indented deeper than the line before it needs a line that opens a block,
/// and a line that is indented less than the line before it needs an outer
/// level of the indentation. A line that goes on with a line that never
/// comes is a line that the interpreter refuses as well.
///
/// A text that does not fit is a text that the interpreter refuses, so it is
/// a text that cannot be tokenised.
fn indentation_fits(text: &str, mask: &[u8]) -> bool {
    let bytes = text.as_bytes();
    // The levels of the indentation that are open.
    let mut levels: Vec<usize> = Vec::new();
    // Whether the line of code that was read last opens a block.
    let mut opens = false;
    // Whether the last line went on with the line that comes now.
    let mut continuing = false;
    // The brackets that are open.
    let mut depth: i64 = 0;
    let mut first = true;
    // The last character of code of the line of code that is being read.
    let mut last_code = 0u8;
    // The code of the line of code that is being read.
    let mut logical_code = 0usize;
    // Whether the line that is read now starts a line of code.
    let mut at_line_start = true;
    let mut offset = 0usize;
    for line in text.split('\n') {
        let start = offset;
        let end = start + line.len();
        offset = end + 1;
        let depth_at_start = depth;
        // The columns of the indentation of the line.
        let mut columns = 0usize;
        let mut index = start;
        while index < end {
            match bytes[index] {
                b' ' => columns += 1,
                b'\t' => columns = (columns / 8 + 1) * 8,
                b'\x0c' => columns = 0,
                _ => break,
            }
            index += 1;
        }
        // The code of the line.
        let mut code = 0usize;
        let mut continued = false;
        for index in start..end {
            if mask[index] & OUTSIDE == 0 {
                continue;
            }
            match bytes[index] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'\\' => {
                    // A line that ends with a backslash goes on with the line
                    // that comes now.
                    continued = index + 1 == end || (index + 2 == end && bytes[index + 1] == b'\r');
                }
                _ => {}
            }
            if bytes[index].is_ascii_whitespace() {
                // A line that holds nothing but spaces holds no code.
                continue;
            }
            code += 1;
            logical_code += 1;
            last_code = bytes[index];
        }
        let inside = continuing || depth_at_start > 0;
        if at_line_start && !inside && code > 0 {
            if first {
                // The first line of a text starts at the left margin.
                if columns != 0 {
                    return false;
                }
                levels.push(columns);
                first = false;
            } else if columns > *levels.last().unwrap_or(&0) {
                // A line that is indented deeper needs a block.
                if !opens {
                    return false;
                }
                levels.push(columns);
            } else if columns < *levels.last().unwrap_or(&0) {
                // A line that is indented less needs an outer level.
                while levels.len() > 1 && *levels.last().unwrap_or(&0) > columns {
                    levels.pop();
                }
                if *levels.last().unwrap_or(&0) != columns {
                    return false;
                }
            }
        }
        // The line of code ends here when the line does not go on, when no
        // bracket is open and when the line does not lie inside a string:
        // the line that comes now starts a new line of code.
        let ends = !continued && depth == 0 && (end >= bytes.len() || mask[end] & OUTSIDE != 0);
        if ends {
            if logical_code > 0 {
                // The line of code ends with a colon, so the line that comes
                // now may be indented deeper.
                opens = last_code == b':';
            }
            last_code = 0;
            logical_code = 0;
        }
        at_line_start = ends;
        continuing = continued;
    }
    // A line that goes on with a line that never comes is a text that the
    // interpreter refuses.
    let trimmed = text.strip_suffix('\n').unwrap_or(text);
    if trimmed.ends_with('\\') && mask[trimmed.len() - 1] & OUTSIDE != 0 {
        return false;
    }
    true
}

/// The logical lines of a python text.
///
/// A line that ends with a backslash goes on with the next line, so the
/// two lines are one line.
///
/// A line that is inside a string is a part of the string, so a string of
/// three quotes holds the lines until the three quotes come back. A line
/// of a comment goes to the end of the line, so a backslash at the end of a
/// comment does not go on with the next line.
///
/// A backslash inside a string escapes the character that follows it, so an
/// escaped quote does not close the string. This holds for a string of three
/// quotes as well, so `\"""` does not close the string.
fn logical_lines(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut index = 0;
    let mut quote: Option<u8> = None;
    let mut triple = false;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'\n' {
                lines.push(std::mem::take(&mut line));
                comment = false;
                index += 1;
                continue;
            }
            let (character, length) = character_at(text, index);
            line.push(character);
            index += length;
            continue;
        }
        if let Some(open) = quote {
            if byte == b'\\' {
                // A backslash escapes the character that follows it, so an
                // escaped quote does not close the string. The escaped
                // character is a whole character, also when it holds more
                // than one byte.
                let (character, length) = character_at(text, index);
                line.push(character);
                index += length;
                if index < bytes.len() {
                    let (character, length) = character_at(text, index);
                    line.push(character);
                    index += length;
                }
                continue;
            }
            if triple {
                if byte == open && is_triple_at(bytes, index) {
                    line.push_str(&text[index..index + 3]);
                    index += 3;
                    quote = None;
                    triple = false;
                    continue;
                }
            } else if byte == open {
                line.push(byte as char);
                index += 1;
                quote = None;
                continue;
            }
            let (character, length) = character_at(text, index);
            line.push(character);
            index += length;
            continue;
        }
        match byte {
            b'\'' | b'"' => {
                if is_triple_at(bytes, index) {
                    line.push_str(&text[index..index + 3]);
                    index += 3;
                    triple = true;
                } else {
                    line.push(byte as char);
                    index += 1;
                    triple = false;
                }
                quote = Some(byte);
            }
            b'#' => {
                comment = true;
                line.push('#');
                index += 1;
            }
            b'\n' => {
                let head = line.trim_end().strip_suffix('\\').map(|head| head.len());
                match head {
                    Some(length) => {
                        line.truncate(length);
                        line.push(' ');
                    }
                    None => lines.push(std::mem::take(&mut line)),
                }
                index += 1;
            }
            _ => {
                let (character, length) = character_at(text, index);
                line.push(character);
                index += length;
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The modules that the statements of a line import.
///
/// A line can hold more than one statement, so the line is split at every
/// semicolon that is not inside a string, and every statement of the line
/// is read on its own.
///
/// A statement can hold an import behind a colon, so the statement is split
/// at every colon as well, and every part behind a colon is read on its own.
fn read_line_imports(line: &str, imports: &mut BTreeSet<String>) {
    for statement in split_statements(line) {
        read_imports(statement, imports);
        for part in split_at_colons(statement).into_iter().skip(1) {
            read_imports(part, imports);
        }
    }
}

/// The parts of a statement, split at every colon.
///
/// The statement is split at every colon that is not inside a string and not
/// inside a comment, so `if True: import subprocess` holds two parts and a
/// colon of a string stays a part of the string.
fn split_at_colons(statement: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < statement.len() {
        let (character, length) = character_at(statement, index);
        match character {
            '\'' | '"' => index = end_of_string(statement, index),
            '#' => break,
            ':' => {
                parts.push(&statement[start..index]);
                index += length;
                start = index;
            }
            _ => index += length,
        }
    }
    parts.push(&statement[start..]);
    parts
}

/// The statements of a line of python.
///
/// The line is split at every semicolon that is not inside a string and
/// not inside a comment, so `import math; import subprocess` holds two
/// statements. A semicolon inside a string is a part of the string, so the
/// line is not split there.
fn split_statements(line: &str) -> Vec<&str> {
    let mut statements = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < line.len() {
        let (character, length) = character_at(line, index);
        match character {
            '\'' | '"' => index = end_of_string(line, index),
            '#' => break,
            ';' => {
                statements.push(&line[start..index]);
                index += length;
                start = index;
            }
            _ => index += length,
        }
    }
    statements.push(&line[start..]);
    statements
}

/// The offset after the string that starts at an offset.
///
/// The quote of the string is at the offset. A string of three quotes ends
/// at the next three quotes that are not escaped, a string of one quote
/// ends at the next quote that is not escaped, and a string that is never
/// closed ends at the end of the text.
fn end_of_string(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let quote = bytes[start];
    if is_triple_at(bytes, start) {
        let mut index = start + 3;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index = escaped_end(text, index);
                continue;
            }
            if bytes[index] == quote && is_triple_at(bytes, index) {
                return index + 3;
            }
            index += 1;
        }
        return bytes.len();
    }
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = escaped_end(text, index),
            byte if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

/// The offset after the character that a backslash escapes.
///
/// The backslash is at the offset. A backslash escapes the character that
/// follows it, so the offset is after that whole character, also when the
/// character holds more than one byte.
fn escaped_end(text: &str, index: usize) -> usize {
    let next = index + 1;
    if next >= text.len() {
        return next;
    }
    let (_, length) = character_at(text, next);
    next + length
}

/// Whether three quotes of the same kind start at an offset.
fn is_triple_at(bytes: &[u8], index: usize) -> bool {
    index + 2 < bytes.len() && bytes[index + 1] == bytes[index] && bytes[index + 2] == bytes[index]
}

/// The character at an offset of a text, and its length in bytes.
fn character_at(text: &str, index: usize) -> (char, usize) {
    match text[index..].chars().next() {
        Some(character) => (character, character.len_utf8()),
        None => ('\u{fffd}', 1),
    }
}

/// The pattern of a literal of python.
///
/// A literal can hold a prefix of `r`, of `u` and of `b` and the
/// combinations of `b` and `r`, so `r"socket"` and `b'subprocess'` are
/// literals as well. The prefix of `f` is no prefix of a literal, because a
/// f-string is built at the time of the call.
const LITERAL: &str =
    r#"(?:[bB][rR]|[rR][bB]|[rR]|[uU]|[bB])?(?:"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*')"#;

/// The call of `__import__` and the call of `importlib.import_module`.
///
/// The call holds the name of the module in a string, so the name of the
/// module is the literal of the match. A literal of python can hold a
/// prefix, so `r"socket"` and `b'subprocess'` are literals as well. The
/// name of the literal is read with its escapes, so the name of the module
/// can be written in more than one way.
static DYNAMIC_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?:__import__|importlib\s*\.\s*import_module)\s*\(\s*(?P<literal>{LITERAL})"#
    ))
    .expect("a pattern that compiles")
});

/// The call of the loader of a module.
///
/// The call starts with the name of the loader, so the place of the call is
/// the place of the match.
static IMPORT_CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:__import__|importlib\s*\.\s*import_module)\s*\(")
        .expect("a pattern that compiles")
});

/// The literal of the first argument of a call of the loader.
///
/// The literal is the plain literal of python, so a f-string is no literal
/// of the call: a f-string is built at the time of the call.
static IMPORT_LITERAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?P<literal>{LITERAL})")).expect("a pattern that compiles")
});

/// The offset of a call of the loader of a module whose name is only known
/// at the time of the call.
///
/// A dynamic import is a call of `__import__` or of
/// `importlib.import_module` whose first argument is no plain string
/// literal. A name in a plain string literal is a name that the scan can
/// read, so such a call is no finding. Every other first argument is only
/// known at the time of the call: a name in a f-string, a name in a
/// variable, a name in a concatenation and a name in a call.
///
/// A call inside a string or inside a comment is no call at all, so it is
/// left out.
pub fn dynamic_import_offset(text: &str) -> Option<usize> {
    let mask = python_code_mask(text)?;
    for found in IMPORT_CALL.find_iter(text) {
        if mask.get(found.start()).copied().unwrap_or(0) & CODE == 0 {
            continue;
        }
        // A name behind a word is no call of the loader, so `my__import__`
        // is no call of the loader.
        if text[..found.start()]
            .chars()
            .next_back()
            .is_some_and(|before| before.is_alphanumeric() || before == '_')
        {
            continue;
        }
        if !is_plain_string_argument(text, found.end()) {
            return Some(found.start());
        }
    }
    None
}

/// Whether the first argument of a call of the loader is a plain string
/// literal.
///
/// A plain string literal is a text in quotes that holds nothing but the
/// name of the module, so `__import__("socket")` names a module that the
/// scan can read.
///
/// Every other first argument is only known at the time of the call: a name
/// in a f-string, a name in a variable, a name in a concatenation and a
/// name in a call.
fn is_plain_string_argument(text: &str, start: usize) -> bool {
    let bytes = text.as_bytes();
    let mut index = start;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let Some(captures) = IMPORT_LITERAL.captures_at(text, index) else {
        return false;
    };
    let Some(literal) = captures.name("literal") else {
        return false;
    };
    if literal.start() != index {
        // The literal starts behind the argument, so the argument holds no
        // literal at all.
        return false;
    }
    // A name that is followed by more text is a concatenation, so it is
    // only known at the time of the call.
    let mut after = literal.end();
    while after < bytes.len() && bytes[after].is_ascii_whitespace() {
        after += 1;
    }
    matches!(bytes.get(after), Some(b')') | Some(b','))
}

/// The modules that a python text imports with a call.
///
/// A call of `__import__` with a string and a call of
/// `importlib.import_module` with a string import a module, so the module
/// is read as an import of the script.
///
/// The string of the call can hold a prefix of python and escapes, so
/// `__import__(r"socket")` holds the module `socket` as well.
///
/// A call inside a string or inside a comment is no call at all, so it is
/// left out.
fn dynamic_imports(text: &str) -> Vec<String> {
    let Some(mask) = python_code_mask(text) else {
        return Vec::new();
    };
    let mut modules = Vec::new();
    for captures in DYNAMIC_IMPORT.captures_iter(text) {
        let Some(found) = captures.get(0) else {
            continue;
        };
        if mask.get(found.start()).copied().unwrap_or(0) & CODE == 0 {
            continue;
        }
        let Some(literal) = captures.name("literal").map(|literal| literal.as_str()) else {
            continue;
        };
        let Some(name) = literal_module_name(literal) else {
            continue;
        };
        modules.push(name);
    }
    modules
}

/// The module of a string literal of a call of the loader.
///
/// A literal of python can hold a prefix: `r`, `u`, `b` and the
/// combinations of `b` and `r`. The prefix is no part of the name of the
/// module.
///
/// The escapes of the literal are decoded, so `"sock\x65t"` names the
/// module `socket` and `"\163ocket"` names the module `socket` as well.
/// The escapes of a raw string are text of the string, so they are not
/// decoded.
///
/// The name of the module is the top level name of the path of the module,
/// so `"os.path"` names the module `os`.
fn literal_module_name(literal: &str) -> Option<String> {
    let mut characters = literal.chars();
    let mut prefix = String::new();
    for character in characters.by_ref() {
        if matches!(character, 'r' | 'R' | 'u' | 'U' | 'b' | 'B') {
            prefix.push(character);
            continue;
        }
        if character != '"' && character != '\'' {
            return None;
        }
        let quote = character;
        let rest: String = characters.collect();
        let inner = rest.strip_suffix(quote)?;
        let name = if prefix.contains('r') || prefix.contains('R') {
            inner.to_string()
        } else {
            decode_escapes(inner)?
        };
        return module_name(&name);
    }
    None
}

/// The text of a string literal with the escapes of python decoded.
///
/// The escapes `\\`, `\'`, `\"`, `\n`, `\r`, `\t`, `\0`, `\a`, `\b`,
/// `\f`, `\v`, `\xNN`, `\uNNNN` and `\UNNNNNNNN` are decoded. An
/// escape of one to three octal digits is decoded as well, so `\163`
/// is the character `s`. An escape that is not known is text of the
/// string, so the backslash stays in front of it. A backslash at the end
/// of the text is no literal at all.
fn decode_escapes(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        let escape = characters.next()?;
        match escape {
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'a' => out.push('\u{7}'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            '\n' => {}
            'x' => out.push(hex_escape(&mut characters, 2)?),
            'u' => out.push(hex_escape(&mut characters, 4)?),
            'U' => out.push(hex_escape(&mut characters, 8)?),
            '0'..='7' => out.push(octal_escape(&mut characters, escape)?),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Some(out)
}

/// The character of a hexadecimal escape of a string.
///
/// The escape holds a fixed number of hexadecimal digits, so a shorter or a
/// longer escape is no escape at all.
fn hex_escape(characters: &mut impl Iterator<Item = char>, length: usize) -> Option<char> {
    let mut value = 0u32;
    for _ in 0..length {
        let digit = characters.next()?.to_digit(16)?;
        value = value * 16 + digit;
    }
    char::from_u32(value)
}

/// The character of an octal escape of a string.
///
/// The escape of python holds one to three octal digits, so `\163` is the
/// character `s` and `\7` is the character of the bell. A digit that is
/// no octal digit ends the escape, so the escape of `\08` is the
/// character of the null byte.
fn octal_escape(
    characters: &mut std::iter::Peekable<std::str::Chars>,
    first: char,
) -> Option<char> {
    let mut value = first.to_digit(8)?;
    for _ in 0..2 {
        let Some(digit) = characters
            .peek()
            .copied()
            .and_then(|digit| digit.to_digit(8))
        else {
            break;
        };
        value = value * 8 + digit;
        characters.next();
    }
    char::from_u32(value)
}

/// The flag of a byte that is code.
///
/// The byte of a string or of a comment is no code, so a search in the code
/// never finds a word of a string. The expression of an f-string is code,
/// because the interpreter runs it.
const CODE: u8 = 1;

/// The flag of a byte that is code outside of a string.
///
/// The lines of a string are no lines of code, so the indentation of the
/// text is read with this flag.
const OUTSIDE: u8 = 2;

/// The code of a python text.
///
/// The result holds, for every byte of the text, the flags of the byte. A
/// byte of a string or of a comment is no code, so a search in the code
/// never finds a word of a string. The expression of an f-string is code,
/// because the interpreter runs it, but it is no code outside of a string.
///
/// The function returns `None` when the text cannot be tokenised: a string
/// that is never closed, a bracket that is never closed or an expression of
/// an f-string that is never closed.
fn python_code_mask(text: &str) -> Option<Vec<u8>> {
    let mut mask = vec![0u8; text.len()];
    scan_code(text, 0, &mut mask, 0, None)?;
    Some(mask)
}

/// The deepest nesting of brackets that the interpreter accepts.
///
/// The tokenizer of CPython refuses a text that nests its brackets deeper
/// than this, so a text that nests them deeper cannot be tokenised.
const MAX_NESTING: i64 = 200;

/// Scan the code of a python text from an offset.
///
/// The scan marks the code bytes in the mask. The depth counts the brackets
/// that are open. The scan of a level ends at the bracket that closes it,
/// and the scan of the outer level ends at the end of the text. The
/// closing bracket of a level is the bracket that closes the bracket that
/// opened the level, so a bracket that closes another bracket is no
/// bracket of the interpreter. The function returns `None` when the text
/// cannot be tokenised.
fn scan_code(
    text: &str,
    index: usize,
    mask: &mut [u8],
    depth: i64,
    closing: Option<u8>,
) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = index;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'\'' | b'"' => {
                let triple = is_triple_at(bytes, index);
                index = if is_fstring(bytes, index) {
                    scan_fstring(text, index, mask, byte, triple)?
                } else {
                    scan_string(text, index, byte, triple)?
                };
            }
            b'\\' => {
                // A backslash goes on with the line that comes now, so
                // nothing but the end of the line may follow it.
                if !ends_the_line(bytes, index) {
                    return None;
                }
                mask[index] |= CODE | OUTSIDE;
                index += 1;
            }
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'(' | b'[' | b'{' => {
                if depth + 1 > MAX_NESTING {
                    return None;
                }
                mask[index] |= CODE | OUTSIDE;
                index = scan_code(
                    text,
                    index + 1,
                    mask,
                    depth + 1,
                    Some(closing_bracket(byte)),
                )?;
            }
            b')' | b']' | b'}' => {
                if depth == 0 {
                    // A closing bracket that closes nothing.
                    return None;
                }
                if closing != Some(byte) {
                    // A closing bracket that closes another bracket: the
                    // interpreter refuses the text.
                    return None;
                }
                mask[index] |= CODE | OUTSIDE;
                // The bracket is closed, so the scan of this level is done.
                return Some(index + 1);
            }
            _ => {
                mask[index] |= CODE | OUTSIDE;
                index += 1;
            }
        }
    }
    if depth != 0 {
        // A bracket that is never closed: the interpreter refuses the text.
        return None;
    }
    Some(index)
}

/// The bracket that closes an opening bracket.
///
/// The bracket that closes a parenthesis is a parenthesis, the bracket that
/// closes a bracket is a bracket and the bracket that closes a brace is a
/// brace.
fn closing_bracket(byte: u8) -> u8 {
    match byte {
        b'(' => b')',
        b'[' => b']',
        _ => b'}',
    }
}

/// Scan a string of a python text from its opening quote.
///
/// The scan returns the index after the closing quote of the string, or
/// `None` when the string is never closed.
fn scan_string(text: &str, index: usize, quote: u8, triple: bool) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = index + if triple { 3 } else { 1 };
    loop {
        if index >= bytes.len() {
            return None;
        }
        let byte = bytes[index];
        if byte == b'\\' {
            index = skip_backslashes(text, index, quote, false);
            continue;
        }
        if triple {
            if byte == quote && is_triple_at(bytes, index) {
                return Some(index + 3);
            }
        } else if byte == quote {
            return Some(index + 1);
        } else if byte == b'\n' {
            // A string that is not a string of three quotes cannot run over
            // a newline, so the string is never closed.
            return None;
        }
        index += 1;
    }
}

/// Skip the backslashes of a string and the character that they stand in
/// front of.
///
/// An odd number of backslashes keeps the quote from ending the string, so
/// the quote is skipped with them. The character that the backslashes stand
/// in front of is text of the string, so it is skipped as well. A brace of
/// an f-string is never skipped, because the expression of an f-string runs
/// even when a backslash stands in front of its brace.
fn skip_backslashes(text: &str, index: usize, quote: u8, fstring: bool) -> usize {
    let bytes = text.as_bytes();
    let mut look = index;
    while look < bytes.len() && bytes[look] == b'\\' {
        look += 1;
    }
    if look < bytes.len() && bytes[look] == quote && (look - index) % 2 == 1 {
        // The quote is escaped, so the string goes on.
        return look + 1;
    }
    if look < bytes.len() && bytes[look] != quote && !(fstring && is_brace(bytes[look])) {
        // The character that the backslashes stand in front of is text of
        // the string.
        return look + 1;
    }
    look
}

/// Whether a backslash ends the line of the code.
///
/// A backslash goes on with the line that comes now, so nothing but the end
/// of the line may follow it. A carriage return may follow it, because a
/// text that ends its lines with a carriage return ends the line with the
/// carriage return and the newline.
fn ends_the_line(bytes: &[u8], index: usize) -> bool {
    match bytes.get(index + 1) {
        Some(b'\n') => true,
        Some(b'\r') => bytes.get(index + 2) == Some(&b'\n'),
        _ => false,
    }
}

/// Whether a byte is a brace of an f-string.
fn is_brace(byte: u8) -> bool {
    byte == b'{' || byte == b'}'
}

/// Scan an f-string of a python text from its opening quote.
///
/// The expression of an f-string is code, because the interpreter runs it.
/// The scan marks the code of the expression in the mask and returns the
/// index after the closing quote of the string, or `None` when the string
/// is never closed.
fn scan_fstring(
    text: &str,
    index: usize,
    mask: &mut [u8],
    quote: u8,
    triple: bool,
) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = index + if triple { 3 } else { 1 };
    loop {
        if index >= bytes.len() {
            return None;
        }
        let byte = bytes[index];
        if byte == b'\\' {
            index = skip_backslashes(text, index, quote, true);
            continue;
        }
        if triple {
            if byte == quote && is_triple_at(bytes, index) {
                return Some(index + 3);
            }
        } else if byte == quote {
            return Some(index + 1);
        }
        if byte == b'\n' && !triple {
            // A string that is not a string of three quotes cannot run over
            // a newline, so the string is never closed.
            return None;
        }
        if byte == b'{' {
            // Two braces in a row are one brace of the text of the string.
            if index + 1 < bytes.len() && bytes[index + 1] == b'{' {
                index += 2;
                continue;
            }
            index = scan_expression(text, index + 1, mask, quote)?;
            continue;
        }
        if byte == b'}' {
            if index + 1 < bytes.len() && bytes[index + 1] == b'}' {
                index += 2;
                continue;
            }
            // A single brace is not allowed in an f-string, so the text
            // cannot be tokenised.
            return None;
        }
        index += 1;
    }
}

/// Scan the expression of an f-string of a python text.
///
/// The scan marks the code of the expression in the mask and returns the
/// index after the closing brace of the expression, or `None` when the
/// expression is never closed.
fn scan_expression(text: &str, index: usize, mask: &mut [u8], quote: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = index;
    // The brackets that are open in the expression. The stack holds the
    // bracket that closes every open bracket, so a bracket that closes
    // another bracket is no bracket of the interpreter.
    let mut open: Vec<u8> = Vec::new();
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'\'' | b'"' => {
                let triple = is_triple_at(bytes, index);
                index = if is_fstring(bytes, index) {
                    scan_fstring(text, index, mask, byte, triple)?
                } else {
                    scan_string(text, index, byte, triple)?
                };
            }
            b'\\' => {
                // A backslash goes on with the line that comes now, so
                // nothing but the end of the line may follow it.
                if !ends_the_line(bytes, index) {
                    return None;
                }
                index += 1;
            }
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'(' | b'[' | b'{' => {
                mask[index] |= CODE;
                open.push(closing_bracket(byte));
                index += 1;
            }
            b')' | b']' => {
                mask[index] |= CODE;
                match open.pop() {
                    Some(closing) if closing == byte => {}
                    // A bracket that closes another bracket: the
                    // interpreter refuses the text.
                    _ => return None,
                }
                index += 1;
            }
            b'}' => {
                match open.last() {
                    // The brace ends the expression of the f-string.
                    None => return Some(index + 1),
                    Some(b'}') => {
                        mask[index] |= CODE;
                        open.pop();
                        index += 1;
                    }
                    // A brace that closes another bracket: the interpreter
                    // refuses the text.
                    Some(_) => return None,
                }
            }
            b':' if open.is_empty() => {
                // The format of the value follows the colon. The format is
                // text of the string, so it is read as text.
                return scan_format(text, index + 1, mask, quote);
            }
            b'!' if open.is_empty() && bytes.get(index + 1) != Some(&b'=') => {
                // The conversion of the value follows the exclamation mark.
                index += 2;
            }
            _ => {
                mask[index] |= CODE;
                index += 1;
            }
        }
    }
    None
}

/// Scan the format of the value of an f-string.
///
/// The format is text of the string, so the characters of the format are no
/// code. The format can hold a replacement field of its own, and the field
/// is code.
///
/// The scan returns the index after the closing brace of the replacement
/// field of the value.
fn scan_format(text: &str, index: usize, mask: &mut [u8], quote: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = index;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => {
                if index + 1 < bytes.len() && bytes[index + 1] == b'{' {
                    index += 2;
                    continue;
                }
                index = scan_expression(text, index + 1, mask, quote)?;
                continue;
            }
            b'}' => {
                // The format of a value holds no brace of its own, so the
                // brace ends the field of the value.
                return Some(index + 1);
            }
            b'\\' => {
                index = skip_backslashes(text, index, quote, true);
            }
            _ => index += 1,
        }
    }
    None
}

/// Whether the string that starts at an offset is an f-string.
///
/// The prefix of a string holds the letters that tell what the string is:
/// the `f` of an f-string, the `r` of a raw string and the `b` of a bytes
/// string.
fn is_fstring(bytes: &[u8], index: usize) -> bool {
    let mut start = index;
    while start > 0 {
        match bytes[start - 1] {
            b'r' | b'R' | b'b' | b'B' | b'u' | b'U' | b'f' | b'F' => start -= 1,
            _ => break,
        }
    }
    bytes[start..index]
        .iter()
        .any(|byte| *byte == b'f' || *byte == b'F')
}

/// The modules that one line of python imports.
///
/// The line is read as a whole, so a line that holds more than one module
/// holds all of them.
fn read_imports(line: &str, imports: &mut BTreeSet<String>) {
    let trimmed = line.trim_start();
    if let Some(rest) = after_keyword(trimmed, "import") {
        for module in rest.split(',') {
            if let Some(name) = module_name(module) {
                imports.insert(name);
            }
        }
    } else if let Some(rest) = after_keyword(trimmed, "from") {
        let module = rest.trim_start();
        if module.starts_with('.') {
            return;
        }
        if let Some(name) = module_name(module) {
            imports.insert(name);
        }
    }
}

/// The text that follows a keyword of a line.
///
/// The keyword is only a keyword when a space follows it, so the line
/// `important = 1` holds no import.
fn after_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim_start())
}

/// The name of the module of an import.
///
/// The name is the top level name of the path of the module, so `d.e` is
/// the module `d`. The alias of an import is left out, so `b as c` is the
/// module `b`. A name that does not start with a letter or an underscore is
/// no module at all.
fn module_name(text: &str) -> Option<String> {
    let text = text.trim_matches(|character| character == '"' || character == '\'');
    let word = text.split_whitespace().next().unwrap_or_default();
    let head = word.split('.').next().unwrap_or_default();
    let mut characters = head.chars();
    let first = characters.next()?;
    if !first.is_ascii_alphabetic() && first != '_' {
        return None;
    }
    let mut name = String::new();
    name.push(first);
    for character in characters {
        if character.is_alphanumeric() || character == '_' {
            name.push(character);
        } else {
            break;
        }
    }
    Some(name)
}

fn collect_links(dir: &Path, skill: &Skill, text: &str) -> Vec<LinkInfo> {
    let mut links: BTreeMap<String, bool> = BTreeMap::new();
    for raw in markdown_link_targets(text) {
        let target = raw.trim();
        if target.is_empty() {
            continue;
        }
        if target.contains("://") || target.starts_with('#') || target.starts_with("mailto:") {
            continue;
        }
        let path_part = target.split('#').next().unwrap_or_default().trim();
        if path_part.is_empty() {
            continue;
        }
        let resolved = normalize_relative(&skill.folder.join(path_part));
        let relative = path_to_string(&resolved);
        let exists = dir.join(&resolved).exists();
        links.entry(relative).or_insert(exists);
    }
    links
        .into_iter()
        .map(|(target, exists)| LinkInfo { target, exists })
        .collect()
}

/// The destinations of the markdown links in `text`.
///
/// The destinations are read with the same parser as the registry cells, so
/// a link that writes its path between angle brackets and a link whose path
/// holds parentheses both read back as the whole path.
fn markdown_link_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut search = 0;
    while let Some(offset) = text[search..].find("](") {
        let start = search + offset + 2;
        match parse_link_destination(&text[start..]) {
            Some((target, length)) => {
                targets.push(target);
                search = start + length;
            }
            None => {
                // The link holds no destination; look for the next link.
                search = start;
            }
        }
    }
    targets
}

fn registry_row(skill: &Skill) -> String {
    let title = skill
        .frontmatter
        .name
        .clone()
        .unwrap_or_else(|| skill.id.clone());
    let title = escape_cell(&title);
    let summary = summary_of(skill.frontmatter.description.as_deref());
    let id = escape_cell(&skill.id);
    let entrypoint = escape_cell(&path_to_string(&skill.entrypoint));
    format!(
        "| {} | {} | {} |",
        title,
        summary,
        link_cell(&id, &entrypoint)
    )
}

/// The markdown link of a registry cell.
///
/// A path that holds a parenthesis, a space or an angle bracket is written
/// between angle brackets, so the markdown parser reads the whole path as
/// the destination of the link.
fn link_cell(label: &str, path: &str) -> String {
    if path.contains(['(', ')', ' ', '<', '>']) {
        format!("[{label}](<{path}>)")
    } else {
        format!("[{label}]({path})")
    }
}

/// Escape a cell so the text stays inside its table cell.
///
/// A backslash is written as `\\` and a pipe as `\|`; the backslash is
/// escaped first, so a text that already holds an escaped pipe keeps its
/// literal pipe. A pipe that is written as `\|` is part of the cell text and
/// never ends the cell, so a title or a summary with a pipe keeps its own
/// cell.
fn escape_cell(text: &str) -> String {
    text.replace('\\', "\\\\").replace('|', "\\|")
}

/// The summary of a registry row: the first sentence of the description.
///
/// The description goes through the redaction helper before the sentence is
/// cut out, so a secret that crosses the cut is never written to the table.
fn summary_of(description: Option<&str>) -> String {
    let Some(text) = description else {
        return "(no description)".to_string();
    };
    let summary = escape_cell(&first_sentence(text, SUMMARY_LIMIT));
    if summary.is_empty() {
        "(no description)".to_string()
    } else {
        summary
    }
}

/// Whether the index file is out of date.
///
/// The fresh index is rendered the same way the command writes it, so the
/// redacted index of the file and the redacted index of the directory are
/// the same when the file is up to date.
fn index_is_stale(path: &Path, catalog: &CatalogIndex) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return true;
    };
    let Ok(committed) = serde_json::from_str::<Value>(&text) else {
        return true;
    };
    let Ok(fresh) = serde_json::to_value(redacted_index(catalog)) else {
        return true;
    };
    committed != fresh
}

fn scan_secrets(dir: &Path) -> Result<Vec<(String, usize)>> {
    let mut files = Vec::new();
    walk_text_files(dir, &mut files)?;
    let mut findings = Vec::new();
    for file in files {
        let Ok(bytes) = fs::read(&file) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        // A type of Python holds no secret, but only in a file of python.
        let python = is_python_file(&file, &text);
        for (index, line) in text.lines().enumerate() {
            if line_has_secret(line, python) {
                findings.push((path_to_string(&relative_to(dir, &file)), index + 1));
            }
        }
    }
    findings.sort();
    Ok(findings)
}

fn walk_text_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(root)
        .with_context(|| format!("read skills directory `{}`", redact_path(root)))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("read skills directory `{}`", redact_path(root)))?;
        paths.push(entry.path());
    }
    paths.sort();

    for path in paths {
        let file_type = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata.file_type(),
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            if let Ok(target) = fs::metadata(&path)
                && target.is_file()
            {
                out.push(path);
            }
            continue;
        }
        if file_type.is_dir() {
            let name = file_name_of(&path);
            if name.starts_with('.') {
                continue;
            }
            walk_text_files(&path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Hide text that looks like a secret before it is printed.
///
/// Every user controlled text that the catalog commands print goes through
/// this helper, so a value that looks like a key never reaches the output.
/// The finding itself keeps the location of the secret, never its value.
///
/// This is the one redaction helper of the crate. Printed values and printed
/// paths alike go through it, so a secret shaped file or directory name is
/// printed as `<redacted>`.
pub fn redact(text: &str) -> String {
    // The text is no code of python, so a value that names a class of
    // python holds a secret here.
    if line_has_secret(text, false) {
        REDACTED.to_string()
    } else {
        text.to_string()
    }
}

/// Hide the secrets of a text, line by line.
///
/// Every line of the text goes through the redaction helper, so a value
/// that looks like a key never reaches the output. A line that holds no
/// secret keeps its text and its line ending, so the text of a file stays
/// the same for every line that holds no secret.
///
/// A line ends with a line feed, with a carriage return, or with both of
/// them, so a file of windows, a file of macos and a file that holds
/// carriage returns only are read line by line as well and keep their line
/// endings.
///
/// A key of a private key holds more than one line, so the lines of such a
/// key are hidden as well: the body of the key is a secret too. The last
/// line of a key is hidden as well, even when it ends without a line
/// ending.
pub fn redact_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // The number of the keys of a private key that are open, so the lines
    // of a key are known while they are read.
    let mut open_keys = 0usize;
    let mut rest = text;
    while !rest.is_empty() {
        // The line is read before it is written, and the state of the keys
        // is read before the line is read, so the last line of a key is
        // hidden as well, even when it ends without a line ending.
        let (content, ending, behind) = split_line(rest);
        rest = behind;
        if open_keys > 0 {
            // The key of a private key holds more than one line, so every
            // line of it is hidden.
            out.push_str(REDACTED);
        } else {
            out.push_str(&redact(content));
        }
        out.push_str(ending);
        // The markers of the line are read in their order, so an end in
        // front of a start never cancels the start: the text behind the
        // line is inside of a key when the last marker of the line opens one.
        open_keys = open_keys_after(content, open_keys);
    }
    out
}

/// Split a text into the text of a line, its line ending, and the text
/// behind it.
///
/// A line ends with a line feed, with a carriage return, or with both of
/// them, so every kind of line ending is found and the line keeps it. The
/// last line of a text has no line ending, so its line ending is empty.
fn split_line(text: &str) -> (&str, &str, &str) {
    let bytes = text.as_bytes();
    for (at, byte) in bytes.iter().enumerate() {
        match *byte {
            b'\n' => return (&text[..at], "\n", &text[at + 1..]),
            b'\r' => {
                if bytes.get(at + 1) == Some(&b'\n') {
                    return (&text[..at], "\r\n", &text[at + 2..]);
                }
                return (&text[..at], "\r", &text[at + 1..]);
            }
            _ => {}
        }
    }
    (text, "", "")
}

/// The number of the keys of a private key that are open behind a line.
///
/// The starts and the ends of the line are read in the order of the line,
/// so `-----END PRIVATE KEY----- -----BEGIN PRIVATE KEY-----` leaves a key
/// open behind the line.
fn open_keys_after(line: &str, open: usize) -> usize {
    let mut markers: Vec<(usize, bool)> = key_marker_positions(line, "-----BEGIN ")
        .into_iter()
        .map(|at| (at, true))
        .chain(
            key_marker_positions(line, "-----END ")
                .into_iter()
                .map(|at| (at, false)),
        )
        .collect();
    markers.sort_unstable();
    markers.into_iter().fold(open, |open, (_, start)| {
        if start {
            open + 1
        } else {
            open.saturating_sub(1)
        }
    })
}

/// The places in a line of the markers of a private key behind a prefix.
///
/// Only the label of a private key counts, so the marker of a certificate
/// is never a marker of a key.
fn key_marker_positions(line: &str, marker: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut search = 0;
    while let Some(offset) = line[search..].find(marker) {
        let at = search + offset;
        let rest = &line[at + marker.len()..];
        if let Some(position) = rest.find("PRIVATE KEY-----")
            && rest[..position]
                .chars()
                .all(|character| character == ' ' || character.is_ascii_uppercase())
        {
            positions.push(at);
        }
        search = at + 1;
    }
    positions
}

/// The number of the starts of a key of a private key in a line.
///
/// Every start of a key is counted, so a line that holds the start and the
/// end of a key at once holds a whole key.
fn count_key_starts(line: &str) -> usize {
    key_marker_positions(line, "-----BEGIN ").len()
}

/// Whether a line holds a secret.
///
/// A type of Python holds no secret, but only in a file of python: an
/// unquoted value of a file of python is a name of python and not a text,
/// so the value of `token: AccessToken` holds no secret. The value of a
/// pair of a file of markdown is a text, so the value of
/// `password: CorrectHorse` holds a secret.
///
/// The check of a line needs to know the language of the file, so the
/// caller tells whether the file holds python code.
pub(crate) fn line_has_secret(line: &str, python: bool) -> bool {
    has_private_key(line)
        || has_aws_key(line)
        || has_openai_key(line)
        || has_sk_key(line)
        || has_github_token(line)
        || has_github_pat(line)
        || has_gitlab_token(line)
        || has_google_key(line)
        || has_slack_token(line)
        || has_generic_secret(line, python)
}

/// The key of an assignment that holds a secret.
///
/// The key is the name of a password, of a token, of a key or of a secret.
static SECRET_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|client[_-]?secret)",
    )
    .expect("a pattern that compiles")
});

/// Whether a line holds a key and a value that look like a secret.
///
/// The key of the assignment is the name of a password, of a token, of a key
/// or of a secret, and the value of the assignment holds at least six
/// characters that are no space.
///
/// Every key and value pair of the line is read, so a secret behind another
/// pair is found as well: the value of the `password:` of
/// `description: password: hunter2xyz` holds a secret.
///
/// The name of the secret has to be a whole word of the key, so a key that
/// holds the name in the middle of a longer word names no secret:
/// `max_tokens` and `tokenizer` name no secret, but `token` and `api_key`
/// do.
///
/// A value that is a placeholder is no secret, so a template of a skill is
/// not a finding: `<token>`, `${token}`, `{{token}}`, `xxxxxx`, `changeme`,
/// `example`, `password`, `secret`, `your_password` and `your-api-key` are
/// placeholders. A value that only holds a placeholder in the middle of it is
/// a secret, so `abcchangeme123` is a secret.
fn has_generic_secret(line: &str, python: bool) -> bool {
    for assignment in assignments(line, is_secret_key) {
        if !is_secret_value(&assignment, python) {
            continue;
        }
        return true;
    }
    false
}

/// Whether the key of an assignment names a secret.
///
/// The name of the secret has to be a whole word of the key, so the name
/// may not sit in the middle of a longer word: `max_tokens`, `query_tokens`
/// and `tokenizer` name no secret, but `token`, `api_key`, `x-api-key`,
/// `API_KEY`, `client_secret` and `POSTGRES_PASSWORD` do.
///
/// A name behind a dash is a name of its own, so `x-api-key` names a
/// secret. A name behind an underscore is a name of its own as well when it
/// is the last part of the key, so `POSTGRES_PASSWORD` names a secret.
fn is_secret_key(key: &str) -> bool {
    for found in SECRET_KEY.find_iter(key) {
        if !is_whole_name(key, found.start(), found.end()) {
            continue;
        }
        return true;
    }
    false
}

/// Whether the name of a secret is a whole word of the key.
///
/// The name may not follow a letter or a digit and it may not be followed by
/// a letter, a digit or an underscore, so the name sits in no longer word.
/// A name behind an underscore is a whole word when nothing follows it, so
/// the name is the last part of the key.
fn is_whole_name(key: &str, start: usize, end: usize) -> bool {
    let before = key[..start].chars().last();
    let after = key[end..].chars().next();
    if before.is_some_and(|character| character.is_alphanumeric()) {
        return false;
    }
    if after.is_some_and(|character| character.is_alphanumeric() || character == '_') {
        return false;
    }
    if before == Some('_') && !key[end..].is_empty() {
        return false;
    }
    true
}

/// Whether a value of an assignment looks like a secret.
///
/// The value holds at least six characters that are no space. A value that
/// is a placeholder is no secret.
///
/// A value that names a shell variable, a value that holds a call or an
/// expression and a value that names a type of Python hold no secret
/// either, so `$KEY`, `os.environ["KEY"]` and `Token = []` are no
/// secrets.
///
/// The whole expression of the value is read, so a call and an expression
/// behind the value are told apart from a value that holds a secret: the
/// value of `password: get_secret (arg)` and the value of
/// `password: lookup = other` hold no secret.
///
/// The expression ends at the end of the assignment, so the fields that
/// follow the value and the brackets that hold the value are no part of
/// the expression: the value of `{password: hunter2xyz, retries: [1, 2]}`
/// holds a secret.
///
/// The expression of a value in quotes goes on behind the closing quote of
/// the value, so a call behind the value holds no secret: the value of
/// `password: "abcdef" + get_secret()` holds no secret.
///
/// The text behind a value that names no code is a text of a document and
/// no code, so such a value holds a secret even when a bracket stands
/// behind it: the value of
/// `description: password: MyRealSecret123 (production)` holds a secret.
///
/// A value that holds a bracket and that is the code itself holds no
/// secret, so the value of `password: get_secret(arg)` holds no secret.
/// A value that holds a bracket and that names no code is a text of a
/// document, so it holds a secret: the value of
/// `description: password: MyRealSecret123(production)` holds a secret.
///
/// A value in quotes is a text and no type of Python, so the value
/// `"Hunter2xyz"` of `password: "Hunter2xyz"` holds a secret.
///
/// The value is read as it is written, so the spaces of a value in quotes
/// are part of the value: the value `" changeme "` is no placeholder and so
/// it is a secret.
fn is_secret_value(assignment: &Assignment, python: bool) -> bool {
    let value = assignment.value.as_str();
    if value
        .chars()
        .filter(|character| !character.is_whitespace())
        .count()
        < 6
    {
        return false;
    }
    if value.starts_with('$') || value.starts_with('%') {
        // A shell variable and an environment variable hold no secret.
        return false;
    }
    if holds_expression(&assignment.value) && is_code_of_a_value(&assignment.value, python) {
        // A value that holds a bracket and that is the code itself is a
        // call or an expression of its own, so the value holds no secret.
        return false;
    }
    if holds_expression(&assignment.expression) && is_code_behind(assignment) {
        // The text behind the value is code and holds a call or an
        // expression, so the value holds no secret. The text behind a
        // value that names no code is a text of a document and no code, so
        // such a value holds a secret all the same.
        return false;
    }
    if holds_closed_placeholder(&assignment.expression) {
        // A closed placeholder and more text behind it hold a secret, so
        // `<token>actual-secret<end>` is a secret.
        return true;
    }
    if python
        && !assignment.quoted
        && is_type_annotation(value, &assignment.expression, assignment.place)
    {
        // A type of Python holds no secret, but only in a file of python.
        return false;
    }
    if is_caps_placeholder(value) {
        // A placeholder in caps holds no secret.
        return false;
    }
    !is_placeholder(value)
}

/// Whether the expression of a value holds a call or an expression.
///
/// A value that holds a bracket holds a call or an expression, and a value
/// that holds an assignment holds an expression as well, so the value holds
/// no secret.
///
/// The value itself is read as well, so a bracket that ends the value is
/// no bracket of the value: the value of `password: value)` holds a
/// bracket and so it holds no secret.
fn holds_expression(expression: &str) -> bool {
    expression
        .chars()
        .any(|character| matches!(character, '(' | ')' | '[' | ']' | '{' | '}'))
        || expression.contains(" = ")
}

/// Whether the text behind the value of an assignment is code.
///
/// The text behind a value is code when it holds an operator, so the value
/// of `password: "abcdef" + get_secret()` holds no secret.
///
/// The text behind a value that is not in quotes is code when the value
/// names the code, so the value of `password: get_secret (arg)` holds no
/// secret.
///
/// The text behind a value that names no code is a text of a document and
/// no code, so a value that is a text holds a secret even when a bracket
/// stands behind it: the value of
/// `description: password: MyRealSecret123 (production)` holds a secret.
fn is_code_behind(assignment: &Assignment) -> bool {
    let behind = text_behind(&assignment.expression, &assignment.value);
    if holds_operator(behind) {
        return true;
    }
    if closes_the_value(behind) {
        return true;
    }
    !assignment.quoted && names_the_code(&assignment.value)
}

/// Whether a text behind a value holds nothing but the brackets that close
/// the value.
///
/// The bracket that closes the call that holds the value is code, so the
/// value of `subTest(token=abcdef123456)` holds no secret.
fn closes_the_value(behind: &str) -> bool {
    !behind.is_empty()
        && behind
            .chars()
            .all(|character| matches!(character, ')' | ']' | '}'))
}

/// The text behind the value of an assignment.
///
/// The expression of an assignment starts with its value, so the text
/// behind the value is the rest of the expression.
fn text_behind<'a>(expression: &'a str, value: &str) -> &'a str {
    expression.strip_prefix(value).unwrap_or(expression)
}

/// Whether a text holds an operator of the code.
///
/// An operator is a character that a text of a document hardly holds, so
/// the text ` + get_secret()` is code and the text ` (production)` is no
/// code.
fn holds_operator(text: &str) -> bool {
    text.chars()
        .any(|character| matches!(character, '+' | '*' | '&' | '|' | '='))
}

/// Whether a value names the code.
///
/// A name of the code is written in small letters, in underscores and in
/// dots, so the value `get_secret` and the value `os.environ` name a call
/// of the code. A value that holds a capital or a digit is a text and no
/// name, so the value `MyRealSecret123` names no code and holds a secret.
///
/// The name is the text in front of the first bracket of the value, so the
/// value `get_secret(arg)` names a call of the code as well.
fn names_the_code(value: &str) -> bool {
    let name = name_in_front(value);
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_' || character == '.')
}

/// The text in front of the first bracket of a text.
///
/// The name of the code is the text in front of the first bracket, so the
/// name of the value `get_secret(arg)` is `get_secret` and the name of the
/// value `MyRealSecret123(production)` is `MyRealSecret123`.
fn name_in_front(value: &str) -> &str {
    value
        .split(['(', '[', '{', ')', ']', '}'])
        .next()
        .unwrap_or(value)
}

/// Whether a value that holds a bracket is the code itself.
///
/// A value that names the code and that holds a bracket is a call of the
/// code, so the value is the code itself: the value `get_secret(arg)` is a
/// call and the value `os.environ["API_KEY"]` is a subscript.
///
/// A value that starts with a bracket is an expression of its own, so the
/// value is the code itself as well: the value `{{token}}actual-secret` is
/// an expression.
///
/// A value that names a class of python and that holds a bracket names a
/// type of python, so the value is the code itself in a file of python: the
/// value `Dict[str, int]` is a type. A file that is no python holds no
/// class, so the value `MyRealSecret123(production)` names no code there
/// and holds a secret.
fn is_code_of_a_value(value: &str, python: bool) -> bool {
    if value.starts_with(['(', '[', '{']) {
        return true;
    }
    if names_the_code(value) {
        return true;
    }
    python && is_class_name(name_in_front(value))
}

/// Whether the expression holds a closed placeholder and more text behind
/// it.
///
/// A placeholder that is closed and that is followed by more text holds a
/// secret, so `<token>actual-secret<end>` is a secret. A value that names
/// a shell variable and a value that holds a brace hold no secret, so
/// `${token}actual-secret` is no secret.
fn holds_closed_placeholder(expression: &str) -> bool {
    for (open, close) in [("<", ">"), ("${", "}"), ("{{", "}}")] {
        let Some(rest) = expression.strip_prefix(open) else {
            continue;
        };
        if let Some(at) = rest.find(close)
            && at + close.len() < rest.len()
        {
            return true;
        }
    }
    false
}

/// Whether the value names a type of Python.
///
/// A type of Python is the name of a class, so the name starts with a
/// capital: the value `Token` of `token: Token = []` names a type and
/// holds no secret, and so does the value `AccessToken` of
/// `token: AccessToken`, because a name of a class can be written in parts
/// and can hold a digit.
///
/// A name that starts with a small letter names no class, so the value
/// `hunter2xyz` of `password: hunter2xyz` holds a secret.
///
/// The type has to stand in an annotation of Python, so the key of the
/// assignment has to start the line or has to sit in a list of arguments.
/// A key that stands behind another pair names a field of a value, and a
/// field of a value holds a secret: the value of the `password:` of
/// `description: password: Hunter2xyz` holds a secret.
///
/// A type of Python holds no secret only in a file of python, because an
/// unquoted value of a file of python is a name of python and not a text.
/// The value of `password: CorrectHorse` of a file of markdown is a text,
/// so it holds a secret.
///
/// The name has to be followed by an equals sign, by a comma, by a bar or
/// by the end of the line, so the value `Token` of `token: Token` names a
/// type as well and so does the value `Token` of `token: Token,`.
///
/// The name has to be a name that is not in quotes, so the value
/// `"Hunter2xyz"` of `password: "Hunter2xyz"` names no type and so it holds
/// a secret.
fn is_type_annotation(value: &str, expression: &str, place: Place) -> bool {
    if place == Place::Nested {
        // A field of a value is no annotation of Python.
        return false;
    }
    if !is_type_name(value) {
        return false;
    }
    let Some(after) = expression.strip_prefix(value) else {
        return false;
    };
    let after = after.trim_start();
    after.is_empty() || after.starts_with('=') || after.starts_with(',') || after.starts_with('|')
}

/// Whether the text is the name of a type of Python.
///
/// A name of a type is the name of a class or a qualified name: a name of
/// a class starts with a capital, so `Token`, `Tokens`, `AccessToken` and
/// `Token2` name a class, and a qualified name names a class of a module,
/// so `datetime.datetime` names a type as well.
///
/// A name that starts with a small letter names no class, so `hunter2xyz`
/// names no type and holds a secret.
///
/// A type can hold a subscript, so `Optional[AccessToken]` names a type as
/// well.
fn is_type_name(value: &str) -> bool {
    let (name, arguments) = match value.split_once('[') {
        Some((name, rest)) => match rest.strip_suffix(']') {
            Some(arguments) => (name, Some(arguments)),
            None => return false,
        },
        None => (value, None),
    };
    if name.is_empty() {
        return false;
    }
    let named = if name.contains('.') {
        // A qualified name names a class of a module.
        name.split('.').all(is_identifier)
    } else {
        // A name of a class starts with a capital.
        is_class_name(name)
    };
    if !named {
        return false;
    }
    match arguments {
        Some(arguments) => !arguments.is_empty(),
        None => true,
    }
}

/// Whether the text is the name of a class of Python.
///
/// A name of a class starts with a capital and holds letters, digits and
/// underscores, so `Token`, `AccessToken` and `Token2` name a class, and
/// `token` and `hunter2xyz` name no class.
fn is_class_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_uppercase() {
        return false;
    }
    characters.all(|character| character.is_alphanumeric() || character == '_')
}

/// Whether the text is the name of python.
///
/// A name of python holds letters, digits and underscores and starts with a
/// letter or with an underscore, so `datetime` and `datetime2` are names of
/// python.
fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    characters.all(|character| character.is_alphanumeric() || character == '_')
}

/// Whether the value is a placeholder that names the secret in caps.
///
/// A template of a skill names the secret in caps, so `YOUR_KEY` and
/// `YOUR_API_KEY` are placeholders and hold no secret.
fn is_caps_placeholder(value: &str) -> bool {
    value.starts_with("YOUR_") && !value.chars().any(|character| character.is_lowercase())
}

/// Whether the whole value of an assignment is a placeholder.
///
/// A placeholder is a value that shows where a secret goes, so the value
/// itself is the placeholder and not a part of it: `<token>`, `${token}`,
/// `{{token}}`, a run of `x` or of `*`, or one of the words `changeme`,
/// `example`, `password`, `secret`, `your_password`, `your_key` and
/// `your-api-key`.
///
/// A value that only holds a placeholder in the middle of it is no
/// placeholder, so `abcchangeme123` is a secret. A value that holds a
/// closed placeholder and more text after it is no placeholder either, so
/// `<token>actual-secret<end>` is a secret.
///
/// The value is read as it is written, so the spaces of a value in quotes
/// are part of the value: the value `" changeme "` is no placeholder.
fn is_placeholder(value: &str) -> bool {
    if placeholder_form(value, "<", ">") {
        return true;
    }
    if placeholder_form(value, "${", "}") {
        return true;
    }
    if placeholder_form(value, "{{", "}}") {
        return true;
    }
    if !value.is_empty()
        && value
            .chars()
            .all(|character| matches!(character, 'x' | 'X' | '*'))
    {
        return true;
    }
    matches!(
        value.to_ascii_lowercase().as_str(),
        "changeme"
            | "example"
            | "password"
            | "secret"
            | "your_password"
            | "your_key"
            | "your-api-key"
    )
}

/// Whether the whole value is a placeholder of a form.
///
/// The value starts with the opening delimiter of the form and the closing
/// delimiter of the form comes at the end of the value, so the value holds
/// nothing but the placeholder. A value that holds a closed placeholder
/// and more text after it is no placeholder, so `<token>actual-secret` is
/// no placeholder.
fn placeholder_form(value: &str, open: &str, close: &str) -> bool {
    let Some(rest) = value.strip_prefix(open) else {
        return false;
    };
    match rest.find(close) {
        Some(at) => at + close.len() == rest.len(),
        None => false,
    }
}

/// The assignments of a line.
///
/// An assignment holds the name of a key, the value of the key, the whole
/// expression of the value and the sign that tells whether the value is in
/// quotes, so a call, an expression and a type of Python are told apart
/// from a value that holds a secret.
struct Assignment {
    /// The value of the key.
    value: String,
    /// The whole expression of the value.
    expression: String,
    /// Whether the value was in quotes.
    quoted: bool,
    /// The place of the key in the line.
    place: Place,
}

/// The place of the key of an assignment in its line.
///
/// The place tells an annotation of Python from a field of a value: the
/// key of an annotation names the target of the annotation, and the key of
/// a field names a field of the value of another key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// The key starts the line, so the assignment is a statement of its
    /// own: the target of an annotation of Python.
    Statement,
    /// The key sits in a list of arguments, so the assignment is a
    /// parameter of a function: the target of an annotation of Python.
    Parameter,
    /// The key stands behind another pair, so the assignment is a field of
    /// the value of another key.
    Nested,
}

/// The place of a key in its line.
///
/// The key of an annotation of Python starts the line or sits in a list of
/// arguments, and the key of a field of a value stands behind another pair.
/// A key in quotes names no target of an annotation, so it names a field of
/// a value.
fn place_of(line: &str, start: usize, quoted: bool) -> Place {
    if quoted {
        return Place::Nested;
    }
    let bytes = line.as_bytes();
    let mut index = start;
    while index > 0 {
        let byte = bytes[index - 1];
        if byte != b' ' && byte != b'\t' {
            break;
        }
        index -= 1;
    }
    if index == 0 {
        // The key starts the text.
        return Place::Statement;
    }
    match bytes[index - 1] {
        // The key starts a line of a text that holds more than one line.
        b'\n' => Place::Statement,
        // The key sits in a list of arguments.
        b'(' | b',' => Place::Parameter,
        // The key stands behind another pair.
        _ => Place::Nested,
    }
}

/// The keys and the values of the assignments of a line.
///
/// A key is a word of letters, of digits, of dashes and of underscores, and
/// the value of the key is the word that follows the colon or the equals
/// sign of the key.
///
/// A key can be a word in quotes as well, so the key of a json file and the
/// key of a yaml file are read as well: `"password": "..."` and
/// `'api_key' = '...'` hold a key and a value.
///
/// The key and the value are read with their escapes decoded, so a key that
/// hides its name behind an escape is the name itself: the key
/// `"pass\u0077ord"` is the key `password` and the value `'ab''hunter2xx'`
/// is the value `ab'hunter2xx`.
///
/// The whole expression of a value that is not in quotes is read as well,
/// so a call and an assignment behind the value are told apart from a value
/// that holds a secret.
///
/// Only the assignments whose key is wanted are read, so the value and the
/// expression of the other keys are never read.
fn assignments<F: Fn(&str) -> bool>(line: &str, wanted: F) -> Vec<Assignment> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    // The brackets that the line holds and the place where the scan of the
    // brackets stopped.
    let mut stack: Vec<u8> = Vec::new();
    let mut scanned = 0usize;
    while index < bytes.len() {
        let (key, key_end, key_start, quoted_key) = if bytes[index] == b'"' || bytes[index] == b'\''
        {
            // A key in quotes holds the word between the quotes.
            let quote = bytes[index];
            let Some(end) = end_of_quoted(bytes, index, quote) else {
                // The quote is never closed, so the line holds no key.
                index += 1;
                continue;
            };
            // A word in quotes is only a key when the colon or the equals
            // sign of an assignment follows it. A word in quotes that is no
            // key is text of the line, so the scan goes on inside of it.
            let mut follows = end;
            while follows < bytes.len() && (bytes[follows] == b' ' || bytes[follows] == b'\t') {
                follows += 1;
            }
            if follows >= bytes.len() || (bytes[follows] != b':' && bytes[follows] != b'=') {
                index += 1;
                continue;
            }
            (
                decode_quoted(&line[index + 1..end - 1], quote),
                end,
                index,
                true,
            )
        } else {
            if !is_key_byte(bytes[index]) || (index > 0 && is_key_byte(bytes[index - 1])) {
                index += 1;
                continue;
            }
            let start = index;
            while index < bytes.len() && is_key_byte(bytes[index]) {
                index += 1;
            }
            (line[start..index].to_string(), index, start, false)
        };
        // The place of the key tells an annotation of Python from a field
        // of a value.
        let place = place_of(line, key_start, quoted_key);
        // The scan goes on behind the key, also when the key is no key of
        // an assignment.
        index = key_end;
        let mut after = key_end;
        while after < bytes.len() && (bytes[after] == b' ' || bytes[after] == b'\t') {
            after += 1;
        }
        if after >= bytes.len() || (bytes[after] != b':' && bytes[after] != b'=') {
            continue;
        }
        let mut value_start = after + 1;
        while value_start < bytes.len()
            && (bytes[value_start] == b' ' || bytes[value_start] == b'\t')
        {
            value_start += 1;
        }
        if !wanted(&key) {
            // The key names no secret, so the value of the key is never
            // read.
            index = after + 1;
            continue;
        }
        // A value in quotes is a text, so the bracket that holds the text
        // holds no value of an assignment.
        let in_quotes = value_start < bytes.len()
            && (bytes[value_start] == b'"' || bytes[value_start] == b'\'');
        // The bracket that holds the value of the assignment.
        let enclosing = if in_quotes {
            None
        } else {
            enclosing_bracket(line, value_start, &mut stack, &mut scanned)
        };
        let (value, value_end, quoted) = if in_quotes {
            let quote = bytes[value_start];
            match end_of_quoted(bytes, value_start, quote) {
                // The value holds the text between the quotes.
                Some(end) => (
                    decode_quoted(&line[value_start + 1..end - 1], quote),
                    end,
                    true,
                ),
                // The quote is never closed, so the value is the rest of
                // the line behind the quote.
                None => (
                    decode_quoted(&line[value_start + 1..], quote),
                    bytes.len(),
                    true,
                ),
            }
        } else {
            let mut end = value_start;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            // The value ends at the delimiter that closes it, so the value
            // of `{password: hunter2xyz, retries: 1}` holds no comma and
            // the value of `{password: hunter2xyz}` holds no bracket.
            let mut word = &line[value_start..end];
            if word.ends_with(',') {
                let at = value_start + word.len() - 1;
                if starts_a_field(line, at + 1) {
                    // A comma that starts the next field is no part of the
                    // value.
                    word = &word[..word.len() - 1];
                }
            }
            if enclosing.is_some() {
                // A bracket that closes the field of the value is no part
                // of the value. The comma that ends the value is removed
                // first, so the bracket behind the comma is found as well.
                word = word.trim_end_matches(['}', ']', ')']);
            }
            (word.to_string(), end, false)
        };
        // The expression of a value goes on behind the value, so a call and
        // an expression behind the value are told apart from a value that
        // holds a secret. The expression of a value in quotes goes on
        // behind the closing quote of the value, so a call behind a value
        // in quotes holds no secret as well.
        let expression = if quoted {
            let mut expression = value.clone();
            expression.push_str(rest_of_expression(line, value_end));
            expression
        } else {
            // A value that is an argument of a call holds the call in its
            // expression, so the expression of such a value ends behind
            // the bracket that closes the call.
            let call = enclosing == Some(b'(');
            expression_of(line, value_start, call)
        };
        if !value.is_empty() {
            found.push(Assignment {
                value,
                expression,
                quoted,
                place,
            });
        }
        index = after + 1;
    }
    found
}

/// The expression of a value that is not in quotes.
///
/// The expression of a value goes on behind the spaces of the value, so a
/// call and an expression behind the value are told apart from a value that
/// holds a secret: the expression of `password: get_secret (arg)` holds a
/// call and the expression of `password: lookup = other` holds an
/// assignment.
///
/// The expression ends at the end of the assignment, so the fields that
/// follow the value and the brackets that hold the value are no part of
/// it: the expression of `{password: hunter2xyz, retries: [1, 2]}` holds
/// no call and no expression.
fn expression_of(line: &str, value_start: usize, call: bool) -> String {
    let end = end_of_expression(line, value_start, call, true);
    line[value_start..end].trim_end().to_string()
}

/// The text behind a value in quotes that belongs to its expression.
///
/// The text ends at the end of the assignment, so the fields that follow
/// the value and the brackets that hold the value are no part of it: the
/// text behind the value of `{"password": "hunter2xx"}` is empty and the
/// text behind the value of `password: "abcdef" + get_secret()` holds a
/// call.
fn rest_of_expression(line: &str, from: usize) -> &str {
    let end = end_of_expression(line, from, false, false);
    line[from..end].trim()
}

/// The end of the expression of a value.
///
/// The expression of a value goes on behind the value, so a call and an
/// expression behind the value are told apart from a value that holds a
/// secret.
///
/// The expression ends at the end of the assignment, so the fields that
/// follow the value and the brackets that hold the value are no part of
/// it: the expression of `{password: hunter2xyz, retries: [1, 2]}` ends
/// behind the value and the expression of `{"password": "hunter2xx"}`
/// ends behind the closing quote of the value.
///
/// A comment that starts behind a space ends the expression as well, so
/// the text of a comment is no part of the expression.
fn end_of_expression(line: &str, start: usize, call: bool, field: bool) -> usize {
    let bytes = line.as_bytes();
    let mut index = start;
    // The brackets that the expression holds. A bracket that closes the
    // field of the value is no bracket of the expression.
    let mut depth = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    if call {
                        // The bracket closes the call that holds the value,
                        // so the expression ends behind it and the call is
                        // no part of the expression.
                        return index + 1;
                    }
                    // The bracket closes the field that holds the value,
                    // so the expression ends in front of it.
                    return index;
                }
                depth -= 1;
            }
            b',' if depth == 0 && !call && (!field || starts_a_field(line, index + 1)) => {
                // The comma starts the next field, so the expression ends
                // in front of it.
                return index;
            }
            b'#' if line[..index]
                .chars()
                .last()
                .is_some_and(|character| character.is_whitespace()) =>
            {
                // A `#` behind a space starts a comment, so the text
                // behind it is no part of the expression.
                return index;
            }
            _ => {}
        }
        index += 1;
    }
    bytes.len()
}

/// The bracket that holds the value of an assignment.
///
/// The scan reads the line until the value and holds the brackets that it
/// passes, so the bracket that holds the value is the last bracket that is
/// still open. A text in quotes holds no bracket, so the scan goes on
/// behind it.
///
/// A bracket that opens a call holds the arguments of the call, so the
/// value of `subTest(token=abcdef123456)` is an argument of a call. Every
/// other bracket holds a field or a text, so the value of
/// `{password: hunter2xyz}` is a field of a mapping.
///
/// The scan holds the place where it stopped, so every bracket of a line
/// is read only once.
fn enclosing_bracket(
    line: &str,
    value_start: usize,
    stack: &mut Vec<u8>,
    scanned: &mut usize,
) -> Option<u8> {
    let bytes = line.as_bytes();
    while *scanned < value_start {
        let index = *scanned;
        match bytes[index] {
            b'"' | b'\'' => {
                // A text in quotes holds no bracket, so the scan goes on
                // behind it.
                match end_of_quoted(bytes, index, bytes[index]) {
                    Some(end) => {
                        // The text ends behind the value, so the scan goes
                        // on behind the text.
                        *scanned = end;
                        continue;
                    }
                    None => {
                        // The quote is never closed, so the rest of the
                        // line is a text and holds no bracket.
                        *scanned = bytes.len();
                        break;
                    }
                }
            }
            b'(' | b'[' | b'{' => stack.push(bytes[index]),
            b')' | b']' | b'}' => {
                stack.pop();
            }
            _ => {}
        }
        *scanned = index + 1;
    }
    stack.last().copied()
}

/// Whether a field of an assignment starts at the given place of a line.
///
/// A field holds a name and a value, so the name is a word of letters, of
/// digits, of dashes and of underscores, or a word in quotes, and the name
/// is followed by a colon or by an equals sign.
///
/// A mapping or a sequence starts the next field of a flow context as
/// well, so the field behind the comma of
/// `[{password: hunter2xyz}, {other: 1}]` starts a field.
fn starts_a_field(line: &str, index: usize) -> bool {
    let bytes = line.as_bytes();
    let mut at = index;
    while at < bytes.len() && (bytes[at] == b' ' || bytes[at] == b'\t') {
        at += 1;
    }
    if at >= bytes.len() {
        return false;
    }
    if bytes[at] == b'{' || bytes[at] == b'[' {
        // A mapping or a sequence starts the next field.
        return true;
    }
    let end = if bytes[at] == b'"' || bytes[at] == b'\'' {
        // The name of the field is in quotes.
        match end_of_quoted(bytes, at, bytes[at]) {
            Some(end) => end,
            None => return false,
        }
    } else {
        let start = at;
        while at < bytes.len() && is_key_byte(bytes[at]) {
            at += 1;
        }
        if at == start {
            return false;
        }
        at
    };
    let mut after = end;
    while after < bytes.len() && (bytes[after] == b' ' || bytes[after] == b'\t') {
        after += 1;
    }
    after < bytes.len() && (bytes[after] == b':' || bytes[after] == b'=')
}

/// The index after the closing quote of a quoted text.
///
/// The text starts at the opening quote. A backslash stands in front of the
/// character that follows it, so the quote behind it does not close the
/// text. Two single quotes in a row are an escaped quote of a single
/// quoted text, so the text goes on behind them.
///
/// The function returns `None` when the text is never closed.
fn end_of_quoted(bytes: &[u8], start: usize, quote: u8) -> Option<usize> {
    let mut index = start + 1;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' {
            // An escaped quote is text of the text, so the scan goes on
            // behind the backslash and the character that it stands in
            // front of.
            index += 2;
            continue;
        }
        if byte == quote {
            if quote == b'\'' && bytes.get(index + 1) == Some(&b'\'') {
                // Two quotes in a row are an escaped quote of a single
                // quoted text, so the text goes on behind them.
                index += 2;
                continue;
            }
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

/// The text of a quoted text, with the escapes of the text decoded.
///
/// A text in double quotes is a text of json, so the escapes of json are
/// decoded: the escape `\u0077` is the letter `w` and the escape `\"`
/// is a quote. A text in single quotes is a text of yaml, so two quotes
/// in a row are an escaped quote and the text holds one quote.
///
/// An escape that is not known is text of the text, so the backslash stays
/// in front of it.
fn decode_quoted(text: &str, quote: u8) -> String {
    if quote == b'\'' {
        // Two quotes in a row are an escaped quote of a single quoted
        // text, so the text holds one quote.
        return text.replace("''", "'");
    }
    decode_json(text)
}

/// The text of a json text, with the escapes of json decoded.
///
/// The escape `\"` is a quote, the escape `\\` is a backslash, the escape
/// `\/` is a slash, the escapes `\b`, `\f`, `\n`, `\r` and `\t` are the
/// characters that they stand for and the escape `\uXXXX` is the character
/// of the four digits.
///
/// A character outside of the basic plane stands for two escapes, so the
/// two escapes are read as one character.
///
/// An escape that is not known is text of the text, so the backslash stays
/// in front of it.
fn decode_json(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let mut characters = rest.chars();
        let Some(escape) = characters.next() else {
            // A backslash at the end of the text is text of the text.
            out.push('\\');
            break;
        };
        rest = characters.as_str();
        match escape {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '/' => out.push('/'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let Some((code, after)) = json_escape(rest) else {
                    // The escape is broken, so the text of the escape stays
                    // as it is.
                    out.push_str("\\u");
                    continue;
                };
                rest = after;
                let code = if (0xd800..0xdc00).contains(&code)
                    && let Some((second, after)) = rest.strip_prefix("\\u").and_then(json_escape)
                    && (0xdc00..0xe000).contains(&second)
                {
                    // The two escapes stand for one character outside of
                    // the basic plane.
                    rest = after;
                    0x10000 + ((code - 0xd800) << 10) + (second - 0xdc00)
                } else {
                    code
                };
                match char::from_u32(code) {
                    Some(character) => out.push(character),
                    None => {
                        // The escape is broken, so the text of the escape
                        // stays as it is.
                        out.push_str("\\u");
                    }
                }
            }
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out.push_str(rest);
    out
}

/// The number of an escape of json and the text behind it.
///
/// The escape holds four hexadecimal digits, so a shorter or a longer
/// number is no escape at all.
fn json_escape(text: &str) -> Option<(u32, &str)> {
    let mut digits = String::new();
    let mut rest = text;
    for _ in 0..4 {
        let character = rest.chars().next()?;
        if !character.is_ascii_hexdigit() {
            return None;
        }
        digits.push(character);
        rest = &rest[character.len_utf8()..];
    }
    let value = u32::from_str_radix(&digits, 16).ok()?;
    Some((value, rest))
}

/// Whether a byte can be a part of the key of an assignment.
fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn has_private_key(line: &str) -> bool {
    count_key_starts(line) > 0
}

fn has_aws_key(line: &str) -> bool {
    let mut search = 0;
    while let Some(offset) = line[search..].find("AKIA") {
        let at = search + offset;
        if has_class_prefix(
            &line[at + 4..],
            |character| character.is_ascii_uppercase() || character.is_ascii_digit(),
            16,
        ) {
            return true;
        }
        search = at + 1;
    }
    false
}

fn has_openai_key(line: &str) -> bool {
    let mut search = 0;
    while let Some(offset) = line[search..].find("sk-") {
        let at = search + offset;
        if has_class_prefix(
            &line[at + 3..],
            |character| character.is_ascii_alphanumeric(),
            20,
        ) {
            return true;
        }
        search = at + 1;
    }
    false
}

fn has_github_token(line: &str) -> bool {
    let mut search = 0;
    while let Some(offset) = line[search..].find("gh") {
        let at = search + offset;
        let rest = &line[at + 2..];
        let Some(kind) = rest.chars().next() else {
            break;
        };
        if matches!(kind, 'p' | 'o' | 'u' | 's' | 'r') {
            let rest = &rest[1..];
            if let Some(rest) = rest.strip_prefix('_')
                && has_class_prefix(rest, |character| character.is_ascii_alphanumeric(), 30)
            {
                return true;
            }
        }
        search = at + 1;
    }
    false
}

fn has_slack_token(line: &str) -> bool {
    let mut search = 0;
    while let Some(offset) = line[search..].find("xox") {
        let at = search + offset;
        let rest = &line[at + 3..];
        let Some(kind) = rest.chars().next() else {
            break;
        };
        if matches!(kind, 'b' | 'a' | 'p' | 'r' | 's') {
            let rest = &rest[1..];
            if let Some(rest) = rest.strip_prefix('-')
                && has_class_prefix(
                    rest,
                    |character| character.is_ascii_alphanumeric() || character == '-',
                    10,
                )
            {
                return true;
            }
        }
        search = at + 1;
    }
    false
}

/// Whether the line holds a `sk-` key such as `sk-proj-…` or `sk-ant-…`.
///
/// The key body holds letters, digits, dashes and underscores; a body that
/// is shorter than twenty characters is not a key.
fn has_sk_key(line: &str) -> bool {
    has_prefixed_body(
        line,
        "sk-",
        |character| character.is_ascii_alphanumeric() || character == '-' || character == '_',
        20,
    )
}

/// Whether the line holds a fine grained GitHub token.
///
/// A fine grained personal access token starts with `github_pat_` and holds
/// at least twenty letters, digits or underscores.
fn has_github_pat(line: &str) -> bool {
    has_prefixed_body(
        line,
        "github_pat_",
        |character| character.is_ascii_alphanumeric() || character == '_',
        20,
    )
}

/// Whether the line holds a GitLab personal access token.
///
/// A GitLab token starts with `glpat-` and holds at least twenty letters,
/// digits, dashes or underscores.
fn has_gitlab_token(line: &str) -> bool {
    has_prefixed_body(
        line,
        "glpat-",
        |character| character.is_ascii_alphanumeric() || character == '-' || character == '_',
        20,
    )
}

/// Whether the line holds a Google API key.
///
/// A Google API key starts with `AIza` and holds thirty five letters, digits,
/// dashes or underscores.
fn has_google_key(line: &str) -> bool {
    has_prefixed_body(
        line,
        "AIza",
        |character| character.is_ascii_alphanumeric() || character == '-' || character == '_',
        35,
    )
}

/// Whether the line holds a token that starts with `prefix`.
///
/// The body after the prefix has to hold at least `count` characters of the
/// given class; a shorter body is not a token.
fn has_prefixed_body(line: &str, prefix: &str, class: impl Fn(char) -> bool, count: usize) -> bool {
    let mut search = 0;
    while let Some(offset) = line[search..].find(prefix) {
        let at = search + offset;
        if has_class_prefix(&line[at + prefix.len()..], &class, count) {
            return true;
        }
        search = at + 1;
    }
    false
}

fn has_class_prefix(text: &str, class: impl Fn(char) -> bool, count: usize) -> bool {
    let mut characters = text.chars();
    for _ in 0..count {
        match characters.next() {
            Some(character) if class(character) => {}
            _ => return false,
        }
    }
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    text: String,
    ending: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistryDocument {
    lines: Vec<Line>,
    header: Option<usize>,
    data_start: Option<usize>,
    data_end: Option<usize>,
}

impl RegistryDocument {
    fn parse(content: &str) -> Self {
        let lines = split_lines(content);
        let fenced = fenced_lines(&lines);
        let mut header = None;
        for (index, line) in lines.iter().enumerate() {
            if fenced[index] {
                continue;
            }
            if line.text.trim() == REGISTRY_HEADER {
                header = Some(index);
                break;
            }
        }

        let mut data_start = None;
        let mut data_end = None;
        if let Some(position) = header {
            // The separator row only counts when it starts with `|`; an indented
            // separator ends the table right after the header.
            let mut start = position + 1;
            if let Some(next) = lines.get(start)
                && !fenced[start]
                && next.text.starts_with('|')
                && is_separator(&next.text)
            {
                start += 1;
            }
            // Data rows end at the first line that does not start with `|`; a
            // fenced code block never holds table rows.
            let mut end = start;
            while end < lines.len() && !fenced[end] && lines[end].text.starts_with('|') {
                end += 1;
            }
            data_start = Some(start);
            data_end = Some(end);
        }

        Self {
            lines,
            header,
            data_start,
            data_end,
        }
    }

    /// The header row index, the first data row index and the end of the
    /// table, when the registry table is present.
    fn table(&self) -> Option<(usize, usize, usize)> {
        match (self.header, self.data_start, self.data_end) {
            (Some(header), Some(start), Some(end)) => Some((header, start, end)),
            _ => None,
        }
    }
}

/// A markdown code fence that opens or closes a fenced code block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fence {
    /// The fence character, either a backtick or a tilde.
    marker: char,
    /// How many times the marker is repeated on the fence line.
    length: usize,
    /// The block container that holds the fence.
    container: Container,
}

/// The block container that holds a fence.
///
/// A fence sits at the top level of the document, in a list item or in a
/// block quote. The container of the opening fence decides where a closing
/// fence has to sit, so a fence in another container never closes the block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Container {
    /// The number of block quote markers in front of the fence.
    quotes: usize,
    /// The column where the content of the container starts; the top level of
    /// the document starts at column zero.
    column: usize,
}

impl Fence {
    /// The fence that opens on `text`, when the line opens a code block.
    ///
    /// A fence is a line that starts with three or more backticks or tildes;
    /// it may be indented by up to three spaces and may live inside a list
    /// item or a block quote.
    ///
    /// A backtick fence may not hold a backtick in its info string: such a
    /// line is inline code, not a fence.
    fn opening(text: &str) -> Option<Self> {
        let prefix = parse_fence_prefix(text)?;
        let rest = prefix.rest;
        let marker = rest.chars().next()?;
        if marker != '`' && marker != '~' {
            return None;
        }
        let length = rest.chars().take_while(|c| *c == marker).count();
        if length < 3 {
            return None;
        }
        // The info string of a backtick fence may not hold a backtick; a
        // line such as "```code``` here" is inline code, not a fence.
        if marker == '`' && rest[length..].contains('`') {
            return None;
        }
        Some(Fence {
            marker,
            length,
            container: Container {
                quotes: prefix.quotes,
                column: prefix.container_column,
            },
        })
    }

    /// Whether `text` closes a block that was opened by this fence.
    ///
    /// The closing fence uses the same character, is at least as long as the
    /// opening fence and holds nothing but the fence itself. It has to sit in
    /// the same container as the opening fence.
    fn closes(&self, text: &str) -> bool {
        let Some(prefix) = parse_fence_prefix(text) else {
            return false;
        };
        if !self.same_container(&prefix) {
            return false;
        }
        let rest = prefix.rest;
        let length = rest.chars().take_while(|c| *c == self.marker).count();
        length >= self.length && rest[length..].trim().is_empty()
    }

    /// Whether the fence of `prefix` sits in the container of the fence that
    /// opened the block.
    ///
    /// The closing fence has to sit in the same block quote and in the same
    /// list item as the opening fence, at the same place inside it. A fence
    /// in another container never closes the block: a line such as "- ```"
    /// inside a top level fence is content of that fence and not a closing
    /// fence.
    fn same_container(&self, prefix: &FencePrefix) -> bool {
        // The closing fence sits in the same block quote.
        if prefix.quotes != self.container.quotes {
            return false;
        }
        // The closing fence does not open a list item of its own; a list
        // marker belongs to another item than the opening fence.
        if prefix.lists != 0 {
            return false;
        }
        // The closing fence sits at the same place inside its container: at
        // most three spaces behind the content of the container.
        prefix.column >= self.container.column && prefix.column <= self.container.column + 3
    }
}

/// The container markers in front of a fence, with the text behind them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FencePrefix<'a> {
    /// The number of block quote markers.
    quotes: usize,
    /// The number of list markers.
    lists: usize,
    /// The column where the content of the innermost container starts.
    container_column: usize,
    /// The column where `rest` starts.
    column: usize,
    /// The text behind the container markers.
    rest: &'a str,
}

/// The container markers in front of a fence and the text behind them.
///
/// A fence may be indented by up to three spaces and may live inside a list
/// item or a block quote, so the list marker and the block quote marker are
/// stripped as well. A deeper indentation is an indented code block and
/// never opens a fence.
///
/// A marker holds the space that belongs to it: the block quote marker is a
/// `>` with the space behind it and the list marker is a dash with the space
/// behind it. The space of a marker is not an indentation, so the three
/// spaces of the indentation are free behind every marker.
fn parse_fence_prefix(text: &str) -> Option<FencePrefix<'_>> {
    let mut rest = text;
    let mut column = 0usize;
    let mut quotes = 0usize;
    let mut lists = 0usize;
    let mut container_column = 0usize;
    loop {
        let spaces = rest.chars().take_while(|c| *c == ' ').count();
        if spaces > 3 {
            return None;
        }
        rest = &rest[spaces..];
        column += spaces;
        // The block quote marker is a `>` with the space behind it; a `>`
        // without a space holds no space of its own.
        if let Some(stripped) = rest.strip_prefix("> ") {
            rest = stripped;
            column += 2;
            quotes += 1;
            container_column = column;
            continue;
        }
        if let Some(stripped) = rest.strip_prefix('>') {
            rest = stripped;
            column += 1;
            quotes += 1;
            container_column = column;
            continue;
        }
        if let Some(width) = list_marker_width(rest) {
            rest = &rest[width..];
            column += width;
            lists += 1;
            container_column = column;
            continue;
        }
        return Some(FencePrefix {
            quotes,
            lists,
            container_column,
            column,
            rest,
        });
    }
}

/// The width of the list marker at the start of `text`, when it holds one.
///
/// The markers are `- `, `* `, `+ ` and an ordered marker such as `1. ` or
/// `1) `.
fn list_marker_width(text: &str) -> Option<usize> {
    for marker in ["- ", "* ", "+ "] {
        if text.starts_with(marker) {
            return Some(marker.len());
        }
    }
    let digits = text.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let rest = &text[digits..];
        for marker in [". ", ") "] {
            if rest.starts_with(marker) {
                return Some(digits + marker.len());
            }
        }
    }
    None
}

/// Mark every line that belongs to a fenced code block.
///
/// The fence markers themselves stay unmarked: they never look like a table
/// row, so only the lines between them have to be skipped.
fn fenced_lines(lines: &[Line]) -> Vec<bool> {
    let mut fenced = vec![false; lines.len()];
    let mut index = 0;
    while index < lines.len() {
        let Some(fence) = Fence::opening(&lines[index].text) else {
            index += 1;
            continue;
        };
        let mut end = index + 1;
        while end < lines.len() && !fence.closes(&lines[end].text) {
            end += 1;
        }
        for slot in fenced.iter_mut().take(end).skip(index + 1) {
            *slot = true;
        }
        // Skip the closing fence; an unclosed fence runs to the end of the
        // document.
        index = end + 1;
    }
    fenced
}

/// Read the registry file; a missing file reads as empty content.
fn read_registry(registry: &Path) -> Result<String> {
    match fs::read_to_string(registry) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => {
            Err(error).with_context(|| format!("read registry `{}`", redact_path(registry)))
        }
    }
}

fn split_lines(content: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for character in content.chars() {
        current.push(character);
        if character == '\n' {
            let ending = if current.ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let text = current[..current.len() - ending.len()].to_string();
            lines.push(Line {
                text,
                ending: ending.to_string(),
            });
            current.clear();
        }
    }
    if !current.is_empty() {
        lines.push(Line {
            text: current,
            ending: String::new(),
        });
    }
    lines
}

fn is_separator(text: &str) -> bool {
    let cells = parse_cells(text);
    !cells.is_empty() && cells.iter().all(|cell| is_separator_cell(cell))
}

/// Whether a cell is a markdown separator cell such as `---` or `:---:`.
///
/// A separator cell is three or more dashes with an optional colon on either
/// side, so an aligned table such as `| :--- | ---: | :---: |` counts too.
fn is_separator_cell(cell: &str) -> bool {
    let cell = cell.trim();
    let core = cell.strip_prefix(':').unwrap_or(cell);
    let core = core.strip_suffix(':').unwrap_or(core);
    core.len() >= 3 && core.chars().all(|character| character == '-')
}

fn parse_cells(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    split_cells(inner)
}

/// Split a table row into its cells.
///
/// Only a pipe that is not escaped as `\|` ends a cell; an escaped pipe stays
/// part of the cell text. A `\\` is a literal backslash and a `\|` is a
/// literal pipe, so a cell reads back as the text it was written from.
fn split_cells(text: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => match characters.next() {
                Some('\\') => current.push('\\'),
                Some('|') => current.push('|'),
                Some(other) => {
                    // An unknown escape keeps its backslash.
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            '|' => {
                cells.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(character),
        }
    }
    cells.push(current.trim().to_string());
    cells
}

/// The destination of the markdown link in a table cell.
///
/// Both `(path)` and `(<path>)` are accepted. A plain destination may hold
/// balanced parentheses, so a path such as `demo(v2)/SKILL.md` reads back as
/// the whole path.
fn extract_link_path(cell: &str) -> Option<String> {
    let start = cell.find("](")? + 2;
    parse_link_destination(&cell[start..]).map(|(path, _)| path)
}

/// The destination of a markdown link and the number of bytes it takes.
///
/// Both `(path)` and `(<path>)` are accepted. A plain destination may hold
/// balanced parentheses, so a path such as `demo(v2)/SKILL.md` reads back as
/// the whole path. The angle brackets around a destination are not part of
/// the path.
fn parse_link_destination(text: &str) -> Option<(String, usize)> {
    if let Some(inner) = text.strip_prefix('<') {
        let end = inner.find('>')?;
        return Some((inner[..end].trim().to_string(), end + 2));
    }
    let mut depth = 0usize;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Some((text[..index].trim().to_string(), index + 1));
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Rewrite the registry file with the kept rows and the appended rows.
///
/// Only the table between the header row and the end of the table changes;
/// every other line keeps its bytes.
fn write_registry(
    document: &RegistryDocument,
    table: (usize, usize, usize),
    kept_rows: &[Line],
    appended: &[String],
    registry: &Path,
) -> Result<()> {
    let (header, start, end) = table;
    let ending = detect_ending(&document.lines);
    let mut lines: Vec<Line> = Vec::new();

    lines.extend(document.lines[..header].iter().cloned());
    lines.extend(document.lines[header..start].iter().cloned());
    lines.extend(kept_rows.iter().cloned());
    for row in appended {
        lines.push(Line {
            text: row.clone(),
            ending: ending.clone(),
        });
    }
    lines.extend(document.lines[end..].iter().cloned());

    let mut content = String::new();
    let last = lines.len().saturating_sub(1);
    for (index, line) in lines.iter().enumerate() {
        let mut line_ending = line.ending.clone();
        if line_ending.is_empty() && index != last {
            line_ending = ending.clone();
        }
        content.push_str(&line.text);
        content.push_str(&line_ending);
    }

    fs::write(registry, content)
        .with_context(|| format!("write registry `{}`", redact_path(registry)))
}

fn detect_ending(lines: &[Line]) -> String {
    for line in lines {
        if !line.ending.is_empty() {
            return line.ending.clone();
        }
    }
    "\n".to_string()
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn relative_to(dir: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(dir).unwrap_or(path).to_path_buf()
}

fn normalize_relative(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = normalized
                    .components()
                    .next_back()
                    .is_some_and(|component| matches!(component, Component::Normal(_)));
                if can_pop {
                    normalized.pop();
                } else {
                    normalized.push("..");
                }
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    normalized
}

fn path_to_string(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => parts.push("..".to_string()),
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    parts.join("/")
}

/// Render a path for a message, with a secret looking path hidden.
///
/// The path goes through the same redaction helper as every other printed
/// value, so a secret shaped file or directory name is printed as
/// `<redacted>`.
pub fn redact_path(path: &Path) -> String {
    redact(&path.display().to_string())
}

fn display_path(dir: &Path, path: &Path) -> String {
    match path.strip_prefix(dir) {
        Ok(relative) => path_to_string(relative),
        Err(_) => {
            let name = file_name_of(path);
            if name.is_empty() {
                path_to_string(path)
            } else {
                name
            }
        }
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_of_a_certificate_never_closes_a_private_key() {
        let line = "-----END CERTIFICATE----- -----BEGIN PRIVATE KEY-----";
        assert_eq!(key_marker_positions(line, "-----END ").len(), 0);
        assert_eq!(count_key_starts(line), 1);
        assert_eq!(
            key_marker_positions("-----END RSA PRIVATE KEY-----", "-----END ").len(),
            1
        );
    }

    #[test]
    fn an_end_in_front_of_a_start_leaves_the_key_open() {
        let line = "-----END PRIVATE KEY----- -----BEGIN PRIVATE KEY-----";
        assert_eq!(open_keys_after(line, 0), 1);
        let text = format!("{line}\nMIIBVQIBADANSECRETBODY\n-----END PRIVATE KEY-----\nafter\n");
        let out = redact_lines(&text);
        assert!(!out.contains("SECRETBODY"), "{out}");
        assert!(out.ends_with("after\n"), "{out}");
        assert_eq!(
            open_keys_after("-----BEGIN PRIVATE KEY----- x -----END PRIVATE KEY-----", 0),
            0
        );
    }
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Whether a line of a file of python holds a secret.
    ///
    /// A type of Python holds no secret in a file of python, because an
    /// unquoted value of such a file is a name of python and not a text.
    fn secret_of_python(line: &str) -> bool {
        line_has_secret(line, true)
    }

    /// Whether a line of a file that holds no python holds a secret.
    ///
    /// The value of a pair of such a file is a text, so it holds a secret
    /// even when it names a class of python.
    fn secret(line: &str) -> bool {
        line_has_secret(line, false)
    }

    const SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

    #[test]
    fn index_lists_folder_and_flat_skills() {
        let dir = fixture("index");
        let index = build_index(&dir).unwrap();

        assert_eq!(index.schema_version, INDEX_SCHEMA_VERSION);
        let ids: Vec<&str> = index.skills.iter().map(|skill| skill.id.as_str()).collect();
        assert_eq!(ids, vec!["bare", "demo", "howto"]);
    }

    #[test]
    fn index_reports_frontmatter_scripts_and_links() {
        let dir = fixture("index");
        let index = build_index(&dir).unwrap();

        let demo = index
            .skills
            .iter()
            .find(|skill| skill.id == "demo")
            .expect("demo skill");
        assert_eq!(demo.kind, SkillKind::Folder);
        assert_eq!(demo.entrypoint, "demo/SKILL.md");
        assert_eq!(demo.name.as_deref(), Some("Demo Skill"));
        assert_eq!(
            demo.description.as_deref(),
            Some("Demo helper. More detail here.")
        );
        assert_eq!(demo.version.as_deref(), Some("1.2.3"));
        assert_eq!(demo.tags, vec!["alpha".to_string(), "beta".to_string()]);
        assert_eq!(demo.requires, vec!["demo-docs".to_string()]);
        assert!(demo.bytes > 0);

        assert_eq!(demo.scripts.len(), 1);
        assert_eq!(demo.scripts[0].path, "demo/scripts/tool.py");
        assert_eq!(demo.scripts[0].language, "python");
        assert_eq!(
            demo.scripts[0].imports,
            vec!["argparse".to_string(), "json".to_string()]
        );

        assert_eq!(demo.links.len(), 2);
        assert!(
            demo.links
                .iter()
                .any(|link| link.target == "demo/docs/guide.md" && link.exists)
        );
        assert!(
            demo.links
                .iter()
                .any(|link| link.target == "demo/docs/missing.md" && !link.exists)
        );
    }

    #[test]
    fn python_imports_keep_unicode_module_names() {
        let imports = python_imports(concat!(
            "import json\n",
            "import caf\u{e9}\n",
            "from na\u{ef}ve import thing\n",
            "import 3bad\n",
        ));

        assert!(imports.contains(&"json".to_string()));
        assert!(imports.contains(&"caf\u{e9}".to_string()));
        assert!(imports.contains(&"na\u{ef}ve".to_string()));
        assert!(!imports.contains(&"caf".to_string()));
        assert!(!imports.contains(&"3bad".to_string()));
    }

    #[test]
    fn python_imports_read_every_module_of_an_import() {
        let imports = python_imports(concat!(
            "import math, subprocess\n",
            "import os.path as path\n",
            "import json\n",
            "from collections import deque\n",
            "import a.b.c\n",
            "    import indented\n",
            "important = 1\n",
            "fromage = 2\n",
        ));

        assert_eq!(
            imports,
            vec![
                "a".to_string(),
                "collections".to_string(),
                "indented".to_string(),
                "json".to_string(),
                "math".to_string(),
                "os".to_string(),
                "subprocess".to_string(),
            ]
        );
    }

    #[test]
    fn python_imports_read_a_statement_that_goes_on_the_next_line() {
        let imports = python_imports(concat!(
            "import math, \\\n",
            "    subprocess\n",
            "from os import \\\n",
            "    path\n",
        ));

        assert_eq!(
            imports,
            vec![
                "math".to_string(),
                "os".to_string(),
                "subprocess".to_string(),
            ]
        );
    }

    #[test]
    fn python_imports_read_an_import_behind_a_colon() {
        let imports = python_imports(concat!(
            "if True: import subprocess\n",
            "try: import socket\n",
            "else: from shutil import copy\n",
        ));

        assert_eq!(
            imports,
            vec![
                "shutil".to_string(),
                "socket".to_string(),
                "subprocess".to_string(),
            ]
        );
    }

    #[test]
    fn python_imports_read_an_import_of_a_call() {
        let imports = python_imports(concat!(
            "module = __import__(\"socket\")\n",
            "other = importlib.import_module('subprocess')\n",
        ));

        assert_eq!(
            imports,
            vec!["socket".to_string(), "subprocess".to_string()]
        );
    }

    #[test]
    fn python_imports_read_a_call_with_a_prefix() {
        // A literal of python can hold a prefix, so the name of the module
        // is read with the prefix.
        let cases = [
            "module = __import__(r\"socket\")\n",
            "module = __import__(R'socket')\n",
            "module = __import__(u\"socket\")\n",
            "module = __import__(b\"socket\")\n",
            "module = __import__(br\"socket\")\n",
            "module = __import__(rb\"socket\")\n",
            "module = __import__(Rb\"socket\")\n",
            "module = importlib.import_module(r\"socket\")\n",
        ];
        for text in cases {
            let imports = python_imports(text);
            assert_eq!(imports, vec!["socket".to_string()], "{text}");
        }
    }

    #[test]
    fn python_imports_read_a_call_with_an_escape() {
        // The escape of the literal is decoded, so the name of the module
        // is the name that the literal holds.
        let imports = python_imports("module = __import__(\"sock\\x65t\")\n");
        assert_eq!(imports, vec!["socket".to_string()]);

        let imports = python_imports("module = __import__(\"sub\\u0070rocess\")\n");
        assert_eq!(imports, vec!["subprocess".to_string()]);
    }

    #[test]
    fn python_imports_read_a_call_with_an_octal_escape() {
        // An escape of one to three octal digits is an escape of python as
        // well, so the name of the module is the name that the literal
        // holds.
        let imports = python_imports("module = __import__(\"\\163ocket\")\n");
        assert_eq!(imports, vec!["socket".to_string()]);

        let imports = python_imports("module = __import__(\"\\160rocess\")\n");
        assert_eq!(imports, vec!["process".to_string()]);

        // An escape of one digit is an escape as well, so the name of the
        // module holds the character of the escape and is no name at all.
        let imports = python_imports("module = __import__(\"\\7ocket\")\n");
        assert!(imports.is_empty(), "{imports:?}");
    }

    #[test]
    fn python_imports_leave_an_f_string_of_a_call_out() {
        // An f-string is no literal, so the name of the module is not a
        // name that the scan can read.
        let imports = python_imports("module = __import__(f\"socket\")\n");
        assert!(imports.is_empty(), "{imports:?}");
    }

    #[test]
    fn python_imports_leave_a_call_of_a_string_out() {
        let imports = python_imports("text = \"__import__(\'socket\')\"\n");

        assert!(imports.is_empty(), "{imports:?}");
    }

    #[test]
    fn an_import_in_an_f_string_is_an_import() {
        // The expression of an f-string is code, so the call of the loader
        // runs when the string is built.
        let imports = python_imports("value = f\"{__import__('socket')}\"\n");
        assert_eq!(imports, vec!["socket".to_string()]);

        let imports = python_imports("value = f\"{__import__(\"socket\")}\"\n");
        assert_eq!(imports, vec!["socket".to_string()]);
    }

    #[test]
    fn a_text_with_a_character_that_cannot_be_printed_is_reported() {
        // The tokenizer of the interpreter refuses a text that holds a null
        // byte or another character that cannot be printed.
        assert!(!python_parses("x = 1\u{0}\n"));
        assert!(!python_parses("x = 1\u{7}\n"));
        assert!(!python_parses("x = 1\u{7f}\n"));
        assert!(python_parses("x = 1\n"));
        assert!(python_parses("x = 1\t\n"));
    }

    #[test]
    fn an_f_string_with_a_single_brace_is_reported() {
        // The interpreter refuses an f-string that holds a single brace.
        assert!(!python_parses("value = f\"}\"\n"));
        assert!(python_parses("value = f\"{{}}\"\n"));
    }

    #[test]
    fn a_string_that_runs_over_a_line_is_reported() {
        // A string that is not a string of three quotes cannot run over a
        // newline, so the text cannot be tokenised.
        assert!(!python_parses("value = \"abc\nimport subprocess\ndef\"\n"));
        // A string of three quotes can run over a newline.
        assert!(python_parses("value = \"\"\"abc\ndef\"\"\"\n"));
        // A backslash in a string goes on with the line that comes now.
        assert!(python_parses("value = \"abc\\\ndef\"\n"));
    }

    #[test]
    fn a_backslash_with_spaces_after_it_is_reported() {
        // Nothing but the end of the line may follow a backslash.
        assert!(!python_parses("value = 1 + \\   \n    2\n"));
        assert!(!python_parses("value = 1 + \\\t\n    2\n"));
        assert!(python_parses("value = 1 + \\\n    2\n"));
        assert!(python_parses("value = 1 + \\\r\n    2\r\n"));
    }

    #[test]
    fn a_format_of_an_f_string_is_no_code() {
        // The format of the value is text of the string, so a hash in the
        // format is no comment.
        assert!(python_parses("value = f\"{value:#x}\"\n"));
        assert!(python_parses("value = f\"{value!r}\"\n"));
        assert!(python_parses("value = f\"{value:>10}\"\n"));
    }

    #[test]
    fn an_import_after_a_backslash_in_an_f_string_is_an_import() {
        // The backslash is text of the string, so the brace still opens the
        // expression.
        let imports = python_imports("value = rf\"\\{__import__('socket')}\"\n");
        assert_eq!(imports, vec!["socket".to_string()]);

        let imports = python_imports("value = f\"\\{__import__('socket')}\"\n");
        assert_eq!(imports, vec!["socket".to_string()]);
    }

    #[test]
    fn a_text_that_the_interpreter_refuses_is_reported() {
        // A line that goes on with a line that never comes.
        assert!(!python_parses("x = 1 \\\n"));
        assert!(python_parses("x = 1 \\\n\n"));
        // A line that is indented deeper than the line before it.
        assert!(!python_parses("x = 1\n    y = 2\n"));
        // A line that is indented less than any level around it.
        assert!(!python_parses("def f():\n    x = 1\n  y = 2\n"));
        // The first line of a text starts at the left margin.
        assert!(!python_parses("  x = 1\n"));
        // A text that the interpreter reads.
        assert!(python_parses("x = 1 + \\\n    2\n"));
        assert!(python_parses("def f():\n    return 1\n"));
        assert!(python_parses("value = f\"{x}\"\n"));
        // A header of a block that goes on with the next line.
        assert!(python_parses("def f(\n    a,\n):\n    return 1\n"));
        // A string that goes on with the next line.
        assert!(python_parses("text = \"\"\"\nhello\n\"\"\"\nvalue = 1\n"));
        // A line that ends with a backslash and a carriage return.
        assert!(python_parses("value = 1 + \\\r\n    2\r\n"));
        // A line that holds nothing but spaces.
        assert!(python_parses("def f():\n    \n    return 1\n"));
        // A text that ends its lines with a carriage return.
        assert!(python_parses("if x:\r\n    pass\r\n"));
    }

    #[test]
    fn a_bracket_that_closes_another_bracket_is_reported() {
        // The interpreter refuses a bracket that closes another bracket.
        assert!(!python_parses("x = ([)]\n"));
        assert!(!python_parses("x = (1, 2]\n"));
        assert!(!python_parses("x = [1, 2)\n"));
        assert!(!python_parses("x = {1: 2]\n"));
        assert!(!python_parses("x = ([{)]}\n"));
        // The interpreter refuses a bracket that is never closed.
        assert!(!python_parses("x = (1, 2\n"));
        assert!(!python_parses("x = [1, 2\n"));
        assert!(!python_parses("x = {1: 2\n"));
        // The brackets of the interpreter fit together.
        assert!(python_parses("x = (1, 2)\n"));
        assert!(python_parses("x = [1, 2]\n"));
        assert!(python_parses("x = {1: 2}\n"));
        assert!(python_parses("x = ([1], {2: 3})\n"));
    }

    #[test]
    fn a_text_that_cannot_be_tokenised_is_reported() {
        assert!(!python_parses("value = \"unterminated\n"));
        assert!(!python_parses("value = (1, 2\n"));
        assert!(python_parses("value = \"closed\"\n"));
        assert!(python_parses("value = (1, 2)\n"));
    }

    #[test]
    fn a_key_and_a_value_that_look_like_a_secret_are_a_secret() {
        assert!(secret("password: hunter2xx"));
        assert!(secret("api_key = \"abcdef123456\""));
        assert!(secret("description: \"Demo password: hunter2xx\""));
        assert!(secret("client_secret=abcdef123456"));
    }

    #[test]
    fn a_key_that_holds_the_name_of_a_secret_in_a_longer_word_is_no_secret() {
        // The name of the secret has to be a whole word of the key, so a
        // key that holds the name in the middle of a longer word names no
        // secret.
        assert!(!secret("max_tokens:4096"));
        assert!(!secret("max_tokens: 409600"));
        assert!(!secret("query_tokens = ..."));
        assert!(!secret("query_tokens = 123456"));
        assert!(!secret("tokenizer = abcdef123456"));
        assert!(!secret("mypassword = abcdef123456"));
        // The text of the line is no secret either, so it is not redacted.
        assert_eq!(redact("max_tokens:4096"), "max_tokens:4096");
        assert_eq!(redact("query_tokens = ..."), "query_tokens = ...");
        assert_eq!(redact("query_tokens = 123456"), "query_tokens = 123456");
        assert_eq!(
            redact("tokenizer = abcdef123456"),
            "tokenizer = abcdef123456"
        );
    }

    #[test]
    fn a_key_that_names_a_secret_as_a_word_of_its_own_is_a_secret() {
        // The name of the secret may be the whole key or the last part of
        // a key of parts.
        assert!(secret("token: hunter2xyz"));
        assert!(secret("api_key = \"sk_live_...\""));
        assert!(secret("API_KEY = \"sk_live_...\""));
        assert!(secret("x-api-key: hunter2xyz"));
        assert!(secret("client_secret=abcdef123456"));
        assert!(secret("POSTGRES_PASSWORD: postgres123"));
    }

    #[test]
    fn a_value_that_holds_no_secret_is_no_secret() {
        // A variable of the shell and a variable of the environment hold
        // no secret.
        assert!(!secret("\"x-api-key\": $key"));
        assert!(!secret("api_key: $API_KEY_VALUE"));
        assert!(!secret("api_key: %API_KEY_VALUE%"));
        // A call and an expression hold no secret.
        assert!(!secret("subTest(token=x)"));
        assert!(!secret("subTest(token=abcdef123456)"));
        assert!(!secret("api_key = os.environ[\"API_KEY\"]"));
        // A value that holds a bracket holds a call or an expression as
        // well.
        assert!(!secret("password: value("));
        assert!(!secret("password: value)"));
        assert!(!secret("password: value["));
        assert!(!secret("password: value]"));
        assert!(!secret("password: value{"));
        assert!(!secret("password: value}"));
        assert!(!secret("password: value = other"));
        // A type of Python holds no secret in a file of python.
        assert!(!secret_of_python("token: Token = []"));
        assert!(!secret_of_python("token: Token"));
        assert!(!secret_of_python("token: Tokens"));
        // A placeholder in caps holds no secret.
        assert!(!secret("x-api-key: YOUR_KEY"));
        assert!(!secret("x-api-key: YOUR_API_KEY"));
        // A short value holds no secret.
        assert!(!secret("api_key: short"));
        // The text of the line is no secret either, so it is not redacted.
        assert_eq!(redact("\"x-api-key\": $key"), "\"x-api-key\": $key");
        assert_eq!(redact("token: Token = []"), "token: Token = []");
        assert_eq!(redact("x-api-key: YOUR_KEY"), "x-api-key: YOUR_KEY");
        assert_eq!(redact("subTest(token=x)"), "subTest(token=x)");
    }

    #[test]
    fn a_key_and_a_value_that_hold_a_secret_are_a_secret() {
        // The key names a secret and the value holds one, so the line is a
        // finding and the text of the line is redacted.
        for line in [
            "password: hunter2xyz",
            "\"password\": \"hunter2xyz\"",
            "api_key = \"sk_live_...\"",
            "POSTGRES_PASSWORD: postgres123",
        ] {
            assert!(secret(line), "{line}");
            assert_eq!(redact(line), REDACTED, "{line}");
        }
    }

    #[test]
    fn a_secret_behind_another_pair_is_a_secret() {
        // Every key and value pair of the line is read, so the value of the
        // `password:` of the line holds a secret, even when another pair
        // comes before it.
        assert!(secret("description: password: MyRealSecret123"));
        assert_eq!(redact("description: password: MyRealSecret123"), REDACTED);
        // The pair behind another pair is no annotation of Python, so the
        // value of the pair holds a secret, even when the value names a
        // class of Python.
        assert!(secret("description: password: Hunter2xyz"));
        assert_eq!(redact("description: password: Hunter2xyz"), REDACTED);
    }

    #[test]
    fn a_type_of_python_holds_no_secret() {
        // A type of Python is the name of a class, so the name starts with
        // a capital. A name can be written in parts and can hold a digit,
        // so `AccessToken` and `Token2` name a class. A qualified name
        // names a class of a module, so `datetime.datetime` names a type.
        assert!(!secret_of_python("token: AccessToken"));
        assert!(!secret_of_python("token: Token"));
        assert!(!secret_of_python("token: Tokens"));
        assert!(!secret_of_python("token: Token2"));
        assert!(!secret_of_python("token: datetime.datetime"));
        assert!(!secret_of_python("token: Optional[AccessToken]"));
        // A name that starts with a small letter names no class, so it
        // holds a secret.
        assert!(secret_of_python("token: accessToken"));
        // A type of Python holds no secret only in a file of python, so
        // the value of a pair of a file of markdown holds a secret, even
        // when it names a class.
        assert!(secret("token: AccessToken"));
        assert!(secret("password: Hunter2xyz"));
        assert_eq!(redact("password: Hunter2xyz"), REDACTED);
    }

    #[test]
    fn a_placeholder_is_no_secret() {
        // The whole value is a placeholder, so the value is no secret.
        assert!(!secret("password: <your-password>"));
        assert!(!secret("password: ${PASSWORD}"));
        assert!(!secret("password: {{ password }}"));
        assert!(!secret("password: xxxxxxxx"));
        assert!(!secret("password: XXXXXXXX"));
        assert!(!secret("password: ********"));
        assert!(!secret("password: changeme"));
        assert!(!secret("password: example"));
        assert!(!secret("password: password"));
        assert!(!secret("password: secret"));
        assert!(!secret("password: your_password"));
        assert!(!secret("password: your-api-key"));
        // The words of a placeholder are placeholders in any case.
        assert!(!secret("password: CHANGEME"));
        assert!(!secret("password: Example"));
        assert!(!secret("password: Your-API-Key"));
        assert!(!secret("password: short"));
    }

    #[test]
    fn a_value_that_only_holds_a_placeholder_is_a_secret() {
        // A placeholder is a placeholder when the whole value is it, so a
        // value that holds a placeholder in the middle of it is a secret.
        assert!(secret("password: abcchangeme123"));
        assert!(secret("password: example-password"));
        assert!(secret("password: your_password_here"));
        assert!(secret("password: xxxxxxxxxxabc"));
    }

    #[test]
    fn a_value_that_holds_a_closed_placeholder_and_a_secret_is_a_secret() {
        // A placeholder is a placeholder when the whole value is it, so a
        // value that holds a closed placeholder and more text after it is
        // a secret.
        assert!(secret("password: <token>actual-secret<end>"));
        assert!(secret("{\"password\": \"<token>actual-secret<end>\"}"));
        // A value that names a shell variable holds no secret, and a value
        // that holds a brace holds an expression, so no secret is in it.
        assert!(!secret("password: ${token}actual-secret"));
        assert!(!secret("password: {{token}}actual-secret"));
        // The whole value is a placeholder, so the value is no secret.
        assert!(!secret("password: <token>"));
        assert!(!secret("password: ${token}"));
        assert!(!secret("password: {{token}}"));
    }

    #[test]
    fn a_value_in_quotes_is_no_type_of_python() {
        // A value in quotes is a text and no type of Python, so the value
        // holds a secret and the text of the line is redacted.
        assert!(secret("password: \"Hunter2xyz\""));
        assert!(secret("\"password\": \"Hunter2xyz\""));
        assert_eq!(redact("password: \"Hunter2xyz\""), REDACTED);
        // A value that is not in quotes and that names a type holds no
        // secret in a file of python.
        assert!(!secret_of_python("token: Token = []"));
        assert!(!secret_of_python("token: Token"));
    }

    #[test]
    fn a_value_in_a_field_of_a_mapping_is_a_secret() {
        // The expression of a value ends at the end of its assignment, so
        // the fields that follow the value and the brackets that hold the
        // value are no part of the expression and the value holds a
        // secret.
        assert!(secret("{password: hunter2xyz, retries: [1, 2]}"));
        assert!(secret("{password: hunter2xyz}"));
        assert!(secret("{\"password\": \"hunter2xx\", \"other\": 1}"));
        // The text of the line is redacted, so the value never reaches the
        // output.
        assert_eq!(redact("{password: hunter2xyz, retries: [1, 2]}"), REDACTED);
    }

    #[test]
    fn a_value_in_a_nested_mapping_is_a_secret() {
        // The bracket that closes the mapping of the value and the field
        // that follows it are no part of the value, so the value holds a
        // secret.
        assert!(secret("{config: {password: hunter2xyz}, retries: 1}"));
        assert!(secret("{config: {password: hunter2xyz}, retries: [1, 2]}"));
        // A sequence of mappings holds the value as well.
        assert!(secret("[{password: hunter2xyz}, {other: 1}]"));
        assert!(secret("[{password: hunter2xyz}]"));
        // The text of the line is redacted, so the value never reaches the
        // output.
        assert_eq!(
            redact("{config: {password: hunter2xyz}, retries: [1, 2]}"),
            REDACTED
        );
        assert_eq!(redact("[{password: hunter2xyz}, {other: 1}]"), REDACTED);
    }

    #[test]
    fn a_call_behind_a_value_in_quotes_holds_no_secret() {
        // The expression of a value in quotes goes on behind the closing
        // quote of the value, so a call behind the value holds no secret.
        assert!(!secret("password: \"abcdef\" + get_secret()"));
        assert!(!secret("password: \"hunter2xx\" + get_secret()"));
        // The text of the line is no secret either, so it is not redacted.
        assert_eq!(
            redact("password: \"abcdef\" + get_secret()"),
            "password: \"abcdef\" + get_secret()"
        );
    }

    #[test]
    fn an_expression_behind_a_value_holds_no_secret() {
        // The whole expression of the value is read, so a call and an
        // assignment behind the value hold no secret.
        assert!(!secret("password: get_secret (arg)"));
        assert!(!secret("password: lookup = other"));
        assert!(!secret("api_key: os.environ[\"API_KEY\"]"));
        // The text of the line is no secret either, so it is not redacted.
        assert_eq!(
            redact("password: get_secret (arg)"),
            "password: get_secret (arg)"
        );
        assert_eq!(
            redact("password: lookup = other"),
            "password: lookup = other"
        );
        // A value that holds a secret is a secret all the same.
        assert!(secret("password: hunter2xyz"));
    }

    #[test]
    fn a_text_behind_a_value_holds_a_secret() {
        // The text behind the value is a text of the document and no code,
        // so the value of the pair holds a secret.
        assert!(secret(
            "description: password: MyRealSecret123 (production)"
        ));
        assert!(secret("description: password: hunter2xyz (production)"));
        assert!(secret("password: MyRealSecret123 (production)"));
        // The text of the line is redacted, so the value never reaches the
        // output.
        assert_eq!(
            redact("description: password: MyRealSecret123 (production)"),
            REDACTED
        );
        // A value that names the code is a call of the code, so it holds no
        // secret.
        assert!(!secret("password: get_secret (arg)"));
        assert!(!secret("description: password: get_secret (arg)"));
        assert!(!secret("password: lookup = other"));
        // A value in quotes is a text, and the code behind it holds no
        // secret.
        assert!(!secret("password: \"abcdef\" + get_secret()"));
        assert!(!secret("password: \"hunter2xx\" + get_secret()"));
    }

    #[test]
    fn a_value_that_holds_a_bracket_and_names_no_code_holds_a_secret() {
        // The value holds a bracket and the value names no code, so the
        // value is a text of a document and holds a secret.
        assert!(secret("description: password: MyRealSecret123(production)"));
        assert!(secret("password: MyRealSecret123(production)"));
        assert!(secret("password: hunter2xyz(production)"));
        // The text of the line is redacted, so the value never reaches the
        // output.
        assert_eq!(
            redact("description: password: MyRealSecret123(production)"),
            REDACTED
        );
        // A value that names the code is a call of the code, so it holds
        // no secret.
        assert!(!secret("password: get_secret(arg)"));
        assert!(!secret("api_key: os.environ[\"API_KEY\"]"));
        assert!(!secret("password: {{token}}actual-secret"));
        // A type of python holds no secret in a file of python, so the
        // value of `token: Dict[str, int]` holds no secret there.
        assert!(!secret_of_python("token: Dict[str, int]"));
    }

    #[test]
    fn a_variable_of_the_shell_holds_no_secret_with_a_text_behind_it() {
        // A value that names a shell variable holds no secret, and a value
        // that holds a brace holds an expression, so the text behind the
        // variable holds no secret.
        assert!(!secret("password: ${TOKEN}suffix"));
        assert!(!secret("password: %TOKEN%suffix"));
        assert_eq!(
            redact("password: ${TOKEN}suffix"),
            "password: ${TOKEN}suffix"
        );
    }

    #[test]
    fn a_key_in_quotes_and_a_value_are_a_secret() {
        // The key of a json file and the key of a yaml file are keys as
        // well, so the value of such a key is a secret.
        assert!(secret("{\"password\": \"hunter2xx\"}"));
        assert!(secret("'api_key' = 'abcdef123456'"));
        assert!(secret("\"token\": \"abcdef123456\""));
        assert!(secret("password: \"hunter2xx\""));
        assert!(!secret("{\"password\": \"short\"}"));
    }

    #[test]
    fn a_value_with_an_escaped_quote_is_a_secret() {
        // An escaped quote is text of the value, so the value goes on
        // behind it and the value is a secret.
        assert!(secret("{\"password\":\"ab\\\"hunter2xx\"}"));
        assert!(secret("password: \"ab\\\"hunter2xx\""));
    }

    #[test]
    fn a_value_with_an_escaped_quote_of_yaml_is_a_secret() {
        // Two quotes in a row are an escaped quote of a yaml text, so the
        // value goes on behind them and the value is a secret.
        assert!(secret("password: 'ab''hunter2xx'"));
        assert!(secret("'api_key': 'ab''hunter2xx'"));
        // A quote that ends the value ends it, so the text behind it is no
        // part of the value.
        assert!(!secret("password: 'ab'"));
    }

    #[test]
    fn a_key_with_an_escaped_letter_is_a_secret() {
        // The escape of a key of json is decoded, so the key hides its name
        // behind the escape and the value of the key is a secret.
        assert!(secret("{\"pass\\u0077ord\": \"hunter2xx\"}"));
        assert!(secret("{\"api\\u005fkey\": \"hunter2xx\"}"));
    }

    #[test]
    fn a_value_with_spaces_around_it_is_no_placeholder() {
        // The spaces of a value in quotes are part of the value, so the
        // value is no placeholder and so it is a secret.
        assert!(secret("password: \" changeme \""));
        assert!(secret("{\"password\": \" changeme \"}"));
        // The whole value is a placeholder, so the value is no secret.
        assert!(!secret("password: changeme"));
        assert!(!secret("{\"password\": \"changeme\"}"));
    }

    #[test]
    fn python_imports_read_every_statement_of_a_line() {
        let imports = python_imports(concat!(
            "import math; import subprocess\n",
            "import json; from collections import deque\n",
            "value = 1; import os\n",
        ));

        assert_eq!(
            imports,
            vec![
                "collections".to_string(),
                "json".to_string(),
                "math".to_string(),
                "os".to_string(),
                "subprocess".to_string(),
            ]
        );
    }

    #[test]
    fn python_imports_do_not_split_a_line_inside_a_string() {
        let imports = python_imports(concat!(
            "text = \"a; import subprocess\"\n",
            "other = 'b; import socket'\n",
            "import json\n",
        ));

        assert_eq!(imports, vec!["json".to_string()]);
    }

    #[test]
    fn python_imports_leave_a_line_of_a_string_of_three_quotes_out() {
        let imports = python_imports(concat!(
            "\"\"\"\n",
            "This is a harmless docstring.\n",
            "a; import subprocess\n",
            "\"\"\"\n",
            "print(1)\n",
        ));

        assert_eq!(imports, Vec::<String>::new());
    }

    #[test]
    fn python_imports_leave_a_string_of_three_quotes_of_one_line_out() {
        let imports = python_imports("x = \"\"\"a; import subprocess\"\"\"\nprint(1)\n");

        assert_eq!(imports, Vec::<String>::new());
    }

    #[test]
    fn python_imports_do_not_split_a_line_inside_a_comment() {
        let imports = python_imports("import json  # a comment; import subprocess\n");

        assert_eq!(imports, vec!["json".to_string()]);
    }

    #[test]
    fn python_imports_leave_a_relative_import_out() {
        let imports = python_imports(concat!(
            "from . import helper\n",
            "from .local import thing\n",
            "from ..other import thing\n",
            "import os\n",
        ));

        assert_eq!(imports, vec!["os".to_string()]);
    }

    #[test]
    fn python_imports_read_a_line_after_a_string_with_a_wide_escape() {
        let imports = python_imports(concat!(
            "text = \"\\\u{e9}\"\n",
            "other = '\\\u{4e2d}'\n",
            "third = \"\\\\\"\n",
            "import subprocess\n",
        ));

        assert_eq!(imports, vec!["subprocess".to_string()]);
    }

    #[test]
    fn python_imports_read_a_line_after_a_string_of_three_quotes_with_an_escape() {
        let imports = python_imports(concat!(
            "x = \"\"\"a \\\"\"\" b\"\"\"\n",
            "import subprocess\n",
        ));

        assert_eq!(imports, vec!["subprocess".to_string()]);
    }

    #[test]
    fn python_imports_read_a_line_after_a_string_of_three_quotes_of_one_kind() {
        let imports = python_imports(concat!("x = '''a \\'' b'''\n", "import subprocess\n",));

        assert_eq!(imports, vec!["subprocess".to_string()]);
    }

    #[test]
    fn python_imports_read_a_line_after_a_string_of_three_quotes_that_goes_on() {
        let imports = python_imports(concat!(
            "x = \"\"\"a \\\n",
            "b\"\"\"\n",
            "import subprocess\n",
        ));

        assert_eq!(imports, vec!["subprocess".to_string()]);
    }

    #[test]
    fn python_imports_survive_every_line_of_a_text_of_quotes_and_wide_characters() {
        let text = concat!(
            "plain = 1\n",
            "double = \"a\\\u{e9}b\"\n",
            "single = '\u{4e2d}'\n",
            "escaped = \"\\\"\"\n",
            "triple = \"\"\"a \\\"\"\" b\"\"\"\n",
            "triple_single = '''a \\' b'''\n",
            "wide = \"\u{1f600}\"\n",
            "backslash = \"\\\\\"\n",
            "comment = 1 # a \" comment\n",
            "import subprocess\n",
        );

        // Every line of the text is read on its own.
        for line in text.lines() {
            let _ = python_imports(line);
        }

        // Every part of the text is read, so every place where a string, an
        // escape or a wide character can sit is read as well.
        let boundaries: Vec<usize> = (0..=text.len())
            .filter(|index| text.is_char_boundary(*index))
            .collect();
        for start in &boundaries {
            for end in &boundaries {
                if start <= end {
                    let _ = python_imports(&text[*start..*end]);
                }
            }
        }

        assert_eq!(python_imports(text), vec!["subprocess".to_string()]);
    }

    #[test]
    fn index_reports_flat_skills_without_scripts() {
        let dir = fixture("index");
        let index = build_index(&dir).unwrap();

        let howto = index
            .skills
            .iter()
            .find(|skill| skill.id == "howto")
            .expect("flat skill");
        assert_eq!(howto.kind, SkillKind::Flat);
        assert_eq!(howto.entrypoint, "howto.md");
        assert!(howto.scripts.is_empty());
        assert!(howto.links.is_empty());
        assert_eq!(howto.name, None);

        let bare = index
            .skills
            .iter()
            .find(|skill| skill.id == "bare")
            .expect("folder skill");
        assert_eq!(bare.kind, SkillKind::Folder);
        assert_eq!(bare.name, None);
        assert_eq!(bare.description, None);
    }

    #[test]
    fn flat_skills_need_a_lowercase_md_extension() {
        assert!(is_flat_skill_file("notes.md"));
        assert!(!is_flat_skill_file("notes.MD"));

        let dir = temp_dir("flat-case");
        write(&dir.join("notes.MD"), "# Notes\n");

        let skills = discover_skills(&dir).unwrap();

        let entrypoints: Vec<String> = skills
            .iter()
            .map(|skill| path_to_string(&skill.entrypoint))
            .collect();
        assert!(entrypoints.is_empty());
    }

    #[test]
    fn index_json_is_deterministic() {
        let dir = fixture("index-deterministic");
        let first = render_index(&build_index(&dir).unwrap()).unwrap();
        let second = render_index(&build_index(&dir).unwrap()).unwrap();

        assert_eq!(first, second);
        assert!(first.ends_with("}\n"));
        let value: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["skills"][0]["id"], "bare");
    }

    #[test]
    fn index_file_is_written_verbatim() {
        let dir = fixture("index-file");
        let index = build_index(&dir).unwrap();
        let rendered = render_index(&index).unwrap();
        let path = dir.join("out/skills.json");
        write_index_file(&path, &rendered).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), rendered);
    }

    #[test]
    fn registry_report_lists_drift_without_writing() {
        let dir = fixture("registry-report");
        let registry = dir.join(REGISTRY_FILE);
        let before = fs::read_to_string(&registry).unwrap();

        let report = reconcile_registry(&dir, &registry, false).unwrap();

        assert!(report.out_of_sync());
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].entrypoint, "ghost/SKILL.md");
        assert_eq!(report.removed[0].reason, "file missing");
        assert_eq!(report.duplicates, vec!["demo/SKILL.md".to_string()]);
        assert_eq!(report.missing.len(), 2);
        assert!(report.missing.iter().any(|row| row.id == "bare"));
        assert!(report.missing.iter().any(|row| row.id == "howto"));
        assert_eq!(report.kept, vec!["demo/SKILL.md".to_string()]);
        assert!(!report.updated);
        assert_eq!(fs::read_to_string(&registry).unwrap(), before);
    }

    #[test]
    fn registry_write_only_touches_the_table() {
        let dir = fixture("registry-write");
        let registry = dir.join(REGISTRY_FILE);
        let report = reconcile_registry(&dir, &registry, true).unwrap();
        assert!(report.updated);

        let after = fs::read_to_string(&registry).unwrap();
        let prefix = concat!(
            "# Skills\n",
            "\n",
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Demo | When demoing | [demo](demo/SKILL.md) |\n"
        );
        assert!(after.starts_with(prefix));
        assert!(after.ends_with("\nTrailing text.\n"));
        assert!(!after.contains("ghost/SKILL.md"));
        assert_eq!(after.matches("demo/SKILL.md").count(), 1);
        assert!(after.contains("| bare | (no description) | [bare](bare/SKILL.md) |"));
        assert!(after.contains("| howto | (no description) | [howto](howto.md) |"));

        let report = reconcile_registry(&dir, &registry, false).unwrap();
        assert!(!report.out_of_sync());
        assert!(report.removed.is_empty());
        assert!(report.duplicates.is_empty());
        assert!(report.missing.is_empty());
    }

    #[test]
    fn registry_without_the_table_is_an_error() {
        let dir = temp_dir("registry-no-table");
        write(&dir.join("demo/SKILL.md"), "# Demo\n");
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            concat!("# Skills\n", "\n", "The registry table is missing.\n",),
        );
        let before = fs::read(&registry).unwrap();

        let error = reconcile_registry(&dir, &registry, true).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("has no `| Skill | When to read | Entrypoint |` table")
        );
        assert_eq!(fs::read(&registry).unwrap(), before);
    }

    #[test]
    fn registry_exit_code_follows_the_command_mode() {
        let dir = fixture("registry-exit-code");
        let registry = dir.join(REGISTRY_FILE);

        let drifted = reconcile_registry(&dir, &registry, false).unwrap();
        assert!(drifted.out_of_sync());
        assert_eq!(drifted.exit_code(false), 1);
        assert_eq!(drifted.exit_code(true), 0);

        let written = reconcile_registry(&dir, &registry, true).unwrap();
        assert!(written.updated);
        assert_eq!(written.exit_code(true), 0);

        let repaired = reconcile_registry(&dir, &registry, false).unwrap();
        assert!(!repaired.out_of_sync());
        assert_eq!(repaired.exit_code(false), 0);
        assert_eq!(repaired.exit_code(true), 0);
    }

    #[test]
    fn registry_missing_rows_use_the_skill_that_owns_the_entrypoint() {
        let dir = temp_dir("registry-missing");
        write(&dir.join("foo.md"), "# Foo\n");
        write(&dir.join("foo/SKILL.md"), "# Foo folder\n");
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            concat!(
                "# Skills\n",
                "\n",
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n"
            ),
        );

        let report = reconcile_registry(&dir, &registry, true).unwrap();

        assert_eq!(report.missing.len(), 2);
        let entrypoints: Vec<&str> = report
            .missing
            .iter()
            .map(|row| row.entrypoint.as_str())
            .collect();
        assert!(entrypoints.contains(&"foo.md"));
        assert!(entrypoints.contains(&"foo/SKILL.md"));

        let after = fs::read_to_string(&registry).unwrap();
        assert!(after.contains("[foo](foo.md)"));
        assert!(after.contains("[foo](foo/SKILL.md)"));
    }

    #[test]
    fn registry_table_ends_at_the_first_line_that_is_not_a_row() {
        let dir = temp_dir("registry-bounds");
        write(&dir.join("demo/SKILL.md"), "# Demo\n");
        write(&dir.join("extra/SKILL.md"), "# Extra\n");
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            concat!(
                "# Skills\n",
                "\n",
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n",
                "| Demo | When demoing | [demo](demo/SKILL.md) |\n",
                "  | indented | not | a table row |\n",
                "\n",
                "Trailing text.\n"
            ),
        );

        let report = reconcile_registry(&dir, &registry, false).unwrap();

        assert!(report.removed.is_empty());
        assert_eq!(report.kept, vec!["demo/SKILL.md".to_string()]);
        assert_eq!(report.missing.len(), 1);

        reconcile_registry(&dir, &registry, true).unwrap();

        let after = fs::read_to_string(&registry).unwrap();
        assert!(after.contains("  | indented | not | a table row |\n"));
        assert!(after.contains("| extra | (no description) | [extra](extra/SKILL.md) |"));
        assert!(after.ends_with("\nTrailing text.\n"));
    }

    #[test]
    fn registry_indented_separator_ends_the_table_at_the_header() {
        let dir = temp_dir("registry-indented-separator");
        write(&dir.join("demo/SKILL.md"), "# Demo\n");
        let registry = dir.join(REGISTRY_FILE);
        let header = "| Skill | When to read | Entrypoint |\n";
        let tail = concat!(
            "  | --- | --- | --- |\n",
            "| Ghost | When ghosting | [ghost](ghost/SKILL.md) |\n",
            "\n",
            "Trailing text.\n",
        );
        write(&registry, &format!("# Skills\n\n{header}{tail}"));

        let report = reconcile_registry(&dir, &registry, true).unwrap();

        assert!(report.updated);
        assert!(report.removed.is_empty());
        assert!(report.kept.is_empty());
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].entrypoint, "demo/SKILL.md");

        // The indented separator is not a table row, so the table ends at the
        // header: the generated row lands right after it and every later line
        // keeps its bytes.
        let inserted = "| demo | (no description) | [demo](demo/SKILL.md) |\n";
        let after = fs::read_to_string(&registry).unwrap();
        assert_eq!(after, format!("# Skills\n\n{header}{inserted}{tail}"));
    }

    #[test]
    fn registry_row_uses_frontmatter_name_and_first_sentence() {
        let dir = temp_dir("registry-row");
        write(
            &dir.join("extra/SKILL.md"),
            "---\nname: Extra Skill\ndescription: First sentence. Second sentence.\n---\n\n# Extra\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            "# Skills\n\n| Skill | When to read | Entrypoint |\n| --- | --- | --- |\n",
        );

        reconcile_registry(&dir, &registry, true).unwrap();

        let after = fs::read_to_string(&registry).unwrap();
        assert!(after.contains("| Extra Skill | First sentence | [extra](extra/SKILL.md) |"));
    }

    #[test]
    fn check_reports_catalog_problems() {
        let dir = fixture("check");
        write(
            &dir.join("demo/secret.txt"),
            "first line\nsecond line\nAKIAIOSFODNN7EXAMPLE\n",
        );
        write(
            &dir.join(INDEX_FILE),
            "{\n  \"schema_version\": 1,\n  \"skills\": []\n}\n",
        );

        let report = check_catalog(&dir, None, None).unwrap();
        let text = report.render();

        assert!(report.has_errors());
        assert!(text.contains("error: skill `bare` is missing frontmatter `name`"));
        assert!(text.contains("error: skill `bare` is missing frontmatter `description`"));
        assert!(text.contains("warning: skill `demo` has frontmatter name `Demo Skill`"));
        assert!(text.contains("error: skill `demo` has a broken link `demo/docs/missing.md`"));
        assert!(text.contains("error: registry `SKILL_REGISTRY.md` is out of sync"));
        assert!(text.contains("error: possible secret in `demo/secret.txt:3`"));
        assert!(text.contains("error: index is stale; run skrun catalog index"));
        assert!(!text.contains(SECRET));
        assert!(text.contains("checked 3 skills:"));
    }

    #[test]
    fn check_reports_a_registry_without_the_table() {
        let dir = temp_dir("check-no-table");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        write(
            &dir.join(REGISTRY_FILE),
            concat!("# Skills\n", "\n", "The registry table is missing.\n",),
        );

        let report = check_catalog(&dir, None, None).unwrap();
        let text = report.render();

        assert!(report.has_errors());
        assert!(text.contains(
            "registry `SKILL_REGISTRY.md` has no `| Skill | When to read | Entrypoint |` table"
        ));
    }

    #[test]
    fn check_passes_for_a_clean_catalog() {
        let dir = temp_dir("check-clean");
        write(
            &dir.join("clean/SKILL.md"),
            "---\nname: clean\ndescription: A clean skill.\n---\n\n# Clean\n\nSee the [guide](docs/guide.md).\n",
        );
        write(&dir.join("clean/docs/guide.md"), "# Guide\n");
        write(
            &dir.join(REGISTRY_FILE),
            concat!(
                "# Skills\n",
                "\n",
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n",
                "| clean | When cleaning | [clean](clean/SKILL.md) |\n"
            ),
        );

        let report = check_catalog(&dir, None, None).unwrap();

        assert!(!report.has_errors(), "{}", report.render());
        assert_eq!(report.warning_count, 0);
        assert_eq!(report.skill_count, 1);
    }

    #[test]
    fn parses_frontmatter_keys_and_lists() {
        let text = concat!(
            "---\n",
            "name: Demo\n",
            "description: Demo helper. More detail.\n",
            "version: 1.0.0\n",
            "tags: alpha, beta\n",
            "requires: [one, two]\n",
            "unknown: ignored\n",
            "---\n",
            "body\n"
        );

        let frontmatter = parse_frontmatter(text);

        assert_eq!(frontmatter.name.as_deref(), Some("Demo"));
        assert_eq!(
            frontmatter.description.as_deref(),
            Some("Demo helper. More detail.")
        );
        assert_eq!(frontmatter.version.as_deref(), Some("1.0.0"));
        assert_eq!(
            frontmatter.tags,
            vec!["alpha".to_string(), "beta".to_string()]
        );
        assert_eq!(
            frontmatter.requires,
            vec!["one".to_string(), "two".to_string()]
        );
    }

    #[test]
    fn keeps_the_commas_of_a_quoted_list_item() {
        let text = concat!(
            "---\n",
            "name: Demo\n",
            "description: Demo helper.\n",
            "tags: [\"one, two\", 'three', four]\n",
            "requires: \"one, two\", three\n",
            "---\n",
            "body\n"
        );

        let frontmatter = parse_frontmatter(text);

        assert_eq!(
            frontmatter.tags,
            vec![
                "one, two".to_string(),
                "three".to_string(),
                "four".to_string()
            ]
        );
        assert_eq!(
            frontmatter.requires,
            vec!["one, two".to_string(), "three".to_string()]
        );
    }

    #[test]
    fn unquotes_the_scalars_of_the_frontmatter() {
        let text = concat!(
            "---\n",
            "name: \"jupyter-notebook\"\n",
            "description: 'A quoted description.'\n",
            "version: \"1.0.0\"\n",
            "tags: [\"one\", 'two']\n",
            "requires: [three]\n",
            "---\n",
            "body\n"
        );

        let frontmatter = parse_frontmatter(text);

        assert_eq!(frontmatter.name.as_deref(), Some("jupyter-notebook"));
        assert_eq!(
            frontmatter.description.as_deref(),
            Some("A quoted description.")
        );
        assert_eq!(frontmatter.version.as_deref(), Some("1.0.0"));
        assert_eq!(frontmatter.tags, vec!["one".to_string(), "two".to_string()]);
        assert_eq!(frontmatter.requires, vec!["three".to_string()]);
    }

    #[test]
    fn unquotes_the_escapes_of_a_double_quoted_scalar() {
        let text = "---\nname: \"a \\\"b\\\" c\"\n---\n";

        let frontmatter = parse_frontmatter(text);

        assert_eq!(frontmatter.name.as_deref(), Some("a \"b\" c"));
    }

    #[test]
    fn unquotes_the_escape_of_a_single_quoted_scalar() {
        let text = "---\nname: 'it''s here'\n---\n";

        let frontmatter = parse_frontmatter(text);

        assert_eq!(frontmatter.name.as_deref(), Some("it's here"));
    }

    #[test]
    fn keeps_a_scalar_that_is_not_wrapped_in_matching_quotes() {
        assert_eq!(unquote_scalar("plain"), "plain");
        assert_eq!(unquote_scalar("\"unbalanced"), "\"unbalanced");
        assert_eq!(unquote_scalar("trailing\""), "trailing\"");
        assert_eq!(unquote_scalar("\"mixed'"), "\"mixed'");
        assert_eq!(unquote_scalar("'"), "'");
        assert_eq!(unquote_scalar(""), "");
    }

    #[test]
    fn the_index_holds_the_unquoted_name_of_a_skill() {
        let dir = temp_dir("index-quoted-name");
        write(
            &dir.join("jupyter/SKILL.md"),
            "---\nname: \"jupyter-notebook\"\ndescription: A notebook.\n---\n\n# Notebook\n",
        );

        let index = build_index(&dir).unwrap();
        let text = render_index(&index).unwrap();

        assert!(text.contains("\"jupyter-notebook\""), "{text}");
        assert!(!text.contains("\\\"jupyter-notebook\\\""), "{text}");
    }

    #[test]
    fn ignores_entries_without_frontmatter() {
        let frontmatter = parse_frontmatter("# Title\n\nbody\n");
        assert_eq!(frontmatter, Frontmatter::default());
    }

    #[test]
    fn frontmatter_needs_exact_delimiter_lines() {
        let frontmatter = parse_frontmatter("---\nname: Demo\n---\n");
        assert_eq!(frontmatter.name.as_deref(), Some("Demo"));

        let indented_start = parse_frontmatter("  ---\nname: Demo\n---\n");
        assert_eq!(indented_start, Frontmatter::default());

        let indented_end = parse_frontmatter("---\nname: Demo\n  ---\n");
        assert_eq!(indented_end, Frontmatter::default());

        let unterminated = parse_frontmatter("---\nname: Demo\ndescription: Demo helper.\n");
        assert_eq!(unterminated, Frontmatter::default());
    }

    #[test]
    fn registry_ignores_fenced_code_blocks() {
        let dir = temp_dir("registry-fence");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        let fence = concat!(
            "```markdown\n",
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Fake | When faking | [fake](fake/SKILL.md) |\n",
            "```\n",
        );
        let table = concat!(
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Ghost | When ghosting | [ghost](ghost/SKILL.md) |\n",
        );
        write(
            &registry,
            &format!("# Skills\n\n{fence}\n{table}\nTrailing text.\n"),
        );

        let report = reconcile_registry(&dir, &registry, true).unwrap();

        // Only the real table is reconciled; the fenced rows are not rows.
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].entrypoint, "ghost/SKILL.md");
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].entrypoint, "demo/SKILL.md");

        let after = fs::read_to_string(&registry).unwrap();
        // The fenced example keeps its bytes.
        assert!(after.contains(fence), "{after}");
        assert!(
            after.contains("| Demo | A demo skill. | [demo](demo/SKILL.md) |"),
            "{after}"
        );
        assert!(after.ends_with("\nTrailing text.\n"), "{after}");
    }

    #[test]
    fn registry_ignores_tilde_and_indented_fences() {
        let dir = temp_dir("registry-tilde-fence");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        let fence = concat!(
            "  ~~~\n",
            "| Skill | When to read | Entrypoint |\n",
            "| Fake | When faking | [fake](fake/SKILL.md) |\n",
            "  ~~~\n",
        );
        let table = concat!(
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Ghost | When ghosting | [ghost](ghost/SKILL.md) |\n",
        );
        write(
            &registry,
            &format!("# Skills\n\n{fence}\n{table}\nTrailing text.\n"),
        );

        let report = reconcile_registry(&dir, &registry, false).unwrap();

        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].entrypoint, "ghost/SKILL.md");
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].entrypoint, "demo/SKILL.md");
    }

    #[test]
    fn registry_accepts_an_aligned_separator_row() {
        let dir = temp_dir("registry-aligned-separator");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        write(
            &dir.join("extra/SKILL.md"),
            "---\nname: Extra\ndescription: An extra skill.\n---\n\n# Extra\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        let separator = "| :--- | ---: | :---: |\n";
        write(
            &registry,
            &format!(
                "# Skills\n\n| Skill | When to read | Entrypoint |\n{separator}| Demo | When demoing | [demo](demo/SKILL.md) |\n"
            ),
        );

        let report = reconcile_registry(&dir, &registry, false).unwrap();

        // The aligned separator is a separator, not a row of its own.
        assert!(report.removed.is_empty());
        assert_eq!(report.kept, vec!["demo/SKILL.md".to_string()]);
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].entrypoint, "extra/SKILL.md");

        reconcile_registry(&dir, &registry, true).unwrap();

        let after = fs::read_to_string(&registry).unwrap();
        // The separator row keeps its bytes.
        assert!(after.contains(separator), "{after}");
        assert!(
            after.contains("| Extra | An extra skill. | [extra](extra/SKILL.md) |"),
            "{after}"
        );
    }

    #[test]
    fn table_cells_read_escaped_pipes_and_backslashes() {
        // An escaped pipe is a literal pipe and never ends the cell.
        let cells = parse_cells(r"| a \| b | c |");
        assert_eq!(cells, vec!["a | b".to_string(), "c".to_string()]);

        // An escaped backslash is a literal backslash.
        let cells = parse_cells(r"| a \\ b | c |");
        assert_eq!(cells, vec![r"a \ b".to_string(), "c".to_string()]);

        // An escaped escaped pipe is a literal backslash and a literal pipe.
        let cells = parse_cells(r"| a \\\| b | c |");
        assert_eq!(cells, vec![r"a \| b".to_string(), "c".to_string()]);

        let cells = parse_cells("| plain | [demo](demo/SKILL.md) |");
        assert_eq!(
            cells,
            vec!["plain".to_string(), "[demo](demo/SKILL.md)".to_string()]
        );
    }

    #[test]
    fn registry_rows_keep_escaped_pipes_inside_a_cell() {
        let dir = temp_dir("registry-escaped-pipe");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            concat!(
                "# Skills\n\n",
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n",
                "| Pipe \\| Skill | When piping | [demo](demo/SKILL.md) |\n",
            ),
        );

        let report = reconcile_registry(&dir, &registry, false).unwrap();

        assert!(!report.out_of_sync());
        assert!(report.removed.is_empty());
        assert_eq!(report.kept, vec!["demo/SKILL.md".to_string()]);
    }

    #[test]
    fn registry_escapes_pipes_in_generated_rows() {
        let dir = temp_dir("registry-pipe");
        write(
            &dir.join("pipe/SKILL.md"),
            "---\nname: Pipe | Skill\ndescription: A | B summary.\n---\n\n# Pipe\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            "# Skills\n\n| Skill | When to read | Entrypoint |\n| --- | --- | --- |\n",
        );

        reconcile_registry(&dir, &registry, true).unwrap();

        let after = fs::read_to_string(&registry).unwrap();
        assert!(
            after.contains("| Pipe \\| Skill | A \\| B summary. | [pipe](pipe/SKILL.md) |"),
            "{after}"
        );

        // The written row parses back into the same three cells.
        let report = reconcile_registry(&dir, &registry, false).unwrap();
        assert!(!report.out_of_sync());
        assert_eq!(report.exit_code(false), 0);
        assert_eq!(report.kept, vec!["pipe/SKILL.md".to_string()]);
    }

    #[test]
    fn redact_hides_secret_shaped_text() {
        assert_eq!(redact("plain text"), "plain text");
        assert_eq!(redact(""), "");
        assert_eq!(redact(SECRET), REDACTED);
        assert_eq!(redact(&format!("name: {SECRET}")), REDACTED);
        assert_eq!(redact(&format!("docs/{SECRET}.md")), REDACTED);
    }

    #[test]
    fn redact_lines_hides_the_secrets_of_a_text() {
        // A line that holds no secret keeps its text and its line ending.
        assert_eq!(redact_lines("plain text\n"), "plain text\n");
        assert_eq!(redact_lines(""), "");
        assert_eq!(redact_lines("a\nb\n"), "a\nb\n");

        // A line that holds a secret is hidden, and the other lines of the
        // text are printed as they are.
        assert_eq!(
            redact_lines(&format!("a\nname: {SECRET}\nb\n")),
            format!("a\n{REDACTED}\nb\n")
        );
        assert_eq!(
            redact_lines("a\npassword: hunter2xyz\nb\n"),
            format!("a\n{REDACTED}\nb\n")
        );

        // The last line of a text has no line ending, and it is hidden all
        // the same.
        assert_eq!(redact_lines(&format!("name: {SECRET}")), REDACTED);
    }

    #[test]
    fn redact_lines_keeps_the_line_endings_of_a_text() {
        // A line of windows keeps its carriage return and its line feed.
        assert_eq!(redact_lines("a\r\nb\r\n"), "a\r\nb\r\n");
        assert_eq!(redact_lines("a\r\n"), "a\r\n");

        // A text that ends without a line ending keeps its text.
        assert_eq!(redact_lines("a"), "a");
    }

    #[test]
    fn redact_lines_hides_the_body_of_a_private_key() {
        // The body of a key of a private key is a secret as well, so every
        // line of the key is hidden.
        let key = concat!(
            "-----BEGIN RSA PRIVATE KEY-----\n",
            "MIIEowIBAAKCAQEAx1234567890\n",
            "-----END RSA PRIVATE KEY-----\n",
        );
        let redacted = redact_lines(key);
        assert!(
            !redacted.contains("MIIEowIBAAKCAQEAx1234567890"),
            "{redacted}"
        );
        assert_eq!(redacted.matches(REDACTED).count(), 3, "{redacted}");

        // The text behind the key is printed as it is.
        assert_eq!(
            redact_lines(&format!("{key}after\n")),
            format!("{REDACTED}\n{REDACTED}\n{REDACTED}\nafter\n")
        );
    }

    #[test]
    fn redact_lines_hides_the_last_line_of_a_private_key() {
        // The last line of a key of a private key has no line ending, and
        // it is hidden all the same, so the material of the key never
        // reaches the output.
        let key = concat!(
            "-----BEGIN RSA PRIVATE KEY-----\n",
            "MIIEowIBAAKCAQEAx1234567890",
        );
        let redacted = redact_lines(key);
        assert!(
            !redacted.contains("MIIEowIBAAKCAQEAx1234567890"),
            "{redacted}"
        );
        assert_eq!(redacted, format!("{REDACTED}\n{REDACTED}"));
    }

    #[test]
    fn redact_lines_hides_a_private_key_that_starts_and_ends_in_one_line() {
        // The start and the end of a key of a private key can be in one
        // line, so the key ends in that line and the lines behind it are no
        // secret of the key and are printed as they are.
        let text = concat!(
            "-----BEGIN RSA PRIVATE KEY----- ",
            "MIIEowIBAAKCAQEAx1234567890 ",
            "-----END RSA PRIVATE KEY-----\n",
            "after\n",
        );
        let redacted = redact_lines(text);
        assert!(
            !redacted.contains("MIIEowIBAAKCAQEAx1234567890"),
            "{redacted}"
        );
        assert_eq!(redacted, format!("{REDACTED}\nafter\n"));
    }

    #[test]
    fn redact_lines_reads_the_lines_of_a_text_with_carriage_returns() {
        // A text that holds carriage returns only is read line by line, so
        // the line that holds a secret is hidden and the other lines keep
        // their text and their line endings.
        let text = "a\rpassword: hunter2xyz\rb\r";
        assert_eq!(redact_lines(text), format!("a\r{REDACTED}\rb\r"));

        // A text that holds carriage returns only and no line feed keeps
        // its carriage returns as well.
        let text = format!("a\r{}\rb", SECRET);
        assert_eq!(redact_lines(&text), format!("a\r{REDACTED}\rb"));
    }

    #[test]
    fn check_redacts_secret_shaped_names_and_links() {
        let dir = temp_dir("check-redact");
        write(
            &dir.join("demo/SKILL.md"),
            concat!(
                "---\n",
                "name: Demo AKIAIOSFODNN7EXAMPLE\n",
                "description: A demo skill.\n",
                "---\n",
                "\n",
                "# Demo\n",
                "\n",
                "Read the [missing page](docs/AKIAIOSFODNN7EXAMPLE.md).\n",
            ),
        );

        let report = check_catalog(&dir, None, None).unwrap();
        let text = report.render();

        assert!(!text.contains(SECRET), "{text}");
        assert!(text.contains("has frontmatter name `<redacted>`"), "{text}");
        assert!(text.contains("has a broken link `<redacted>`"), "{text}");
        // The secret finding keeps the location, never the value.
        assert!(
            text.contains("possible secret in `demo/SKILL.md:2`"),
            "{text}"
        );
    }

    #[test]
    fn registry_report_redacts_secret_shaped_rows() {
        let dir = temp_dir("registry-report-redact");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            concat!(
                "# Skills\n\n",
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n",
                "| Broken | When breaking | AKIAIOSFODNN7EXAMPLE |\n",
            ),
        );

        let report = reconcile_registry(&dir, &registry, false).unwrap();
        let text = report.render();

        assert!(!text.contains(SECRET), "{text}");
        assert!(
            text.contains("stale row (invalid entrypoint cell): <redacted>"),
            "{text}"
        );
    }

    #[test]
    fn check_redacts_secret_shaped_paths() {
        let dir = temp_dir("check-redact-path");
        // A file whose name looks like a secret.
        write(
            &dir.join(format!("{SECRET}.md")),
            "the key is AKIAIOSFODNN7EXAMPLE\n",
        );
        // A directory whose name looks like a secret.
        write(
            &dir.join(format!("{SECRET}-dir/notes.md")),
            "the key is AKIAIOSFODNN7EXAMPLE\n",
        );

        let report = check_catalog(&dir, None, None).unwrap();
        let text = report.render();

        assert!(!text.contains(SECRET), "{text}");
        assert!(text.contains("<redacted>"), "{text}");
        // The finding keeps the line, never the secret shaped name.
        assert!(text.contains("possible secret in `<redacted>:1`"), "{text}");
    }

    #[test]
    fn registry_escapes_backslashes_in_generated_rows() {
        let dir = temp_dir("registry-backslash");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Back \\ Slash | Pipe\ndescription: A \\ B summary.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            "# Skills\n\n| Skill | When to read | Entrypoint |\n| --- | --- | --- |\n",
        );

        reconcile_registry(&dir, &registry, true).unwrap();

        let after = fs::read_to_string(&registry).unwrap();
        assert!(
            after.contains(r"| Back \\ Slash \| Pipe | A \\ B summary. | [demo](demo/SKILL.md) |"),
            "{after}"
        );

        // The written row reads back as the same three cells.
        let report = reconcile_registry(&dir, &registry, false).unwrap();
        assert!(!report.out_of_sync());
        assert_eq!(report.exit_code(false), 0);
        assert_eq!(report.kept, vec!["demo/SKILL.md".to_string()]);
    }

    #[test]
    fn registry_round_trips_a_title_with_pipes_and_backslashes() {
        let dir = temp_dir("registry-round-trip");
        write(
            &dir.join("demo/SKILL.md"),
            concat!(
                "---\n",
                "name: Pipe \\| Skill | Second\n",
                "description: A summary with \\| and |.\n",
                "---\n",
                "\n",
                "# Demo\n",
            ),
        );
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            "# Skills\n\n| Skill | When to read | Entrypoint |\n| --- | --- | --- |\n",
        );

        reconcile_registry(&dir, &registry, true).unwrap();

        let after = fs::read_to_string(&registry).unwrap();
        assert!(
            after.contains(r"| Pipe \\\| Skill \| Second | A summary with \\\| and \|. | [demo](demo/SKILL.md) |"),
            "{after}"
        );

        // The written row reads back as the same three cells, so the table
        // is in sync and a dry run succeeds.
        let report = reconcile_registry(&dir, &registry, false).unwrap();
        assert!(!report.out_of_sync());
        assert_eq!(report.exit_code(false), 0);
        assert_eq!(report.kept, vec!["demo/SKILL.md".to_string()]);
    }

    #[test]
    fn detects_secret_shapes() {
        assert!(secret("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(secret("-----BEGIN PRIVATE KEY-----"));
        assert!(secret("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(secret(SECRET));
        assert!(secret("sk-abcdefghijklmnopqrstuvwxyz012345"));
        assert!(secret("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(secret("gho_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(secret("xoxb-1234567890-abcdefghij"));
        assert!(!secret("nothing to see here"));
        assert!(!secret("sk-short"));
        assert!(!secret("AKIASHORT"));
        assert!(!secret("-----BEGIN CERTIFICATE-----"));
        assert!(!secret("risk-analysis-notes"));
    }

    #[test]
    fn secret_matchers_respect_the_minimum_length() {
        assert!(secret("AKIA0123456789ABCDEF"));
        assert!(!secret("AKIA0123456789ABCDE"));
        assert!(secret("sk-012345678901234567890"));
        assert!(!secret("sk-0123456789012345678"));
        assert!(secret("ghp_012345678901234567890123456789"));
        assert!(!secret("ghp_01234567890123456789012345678"));
        assert!(secret("xoxb-0123456789"));
        assert!(!secret("xoxb-012345678"));
    }

    #[test]
    fn detects_the_new_secret_shapes() {
        // A project key, an application key and every other `sk-` key.
        assert!(secret("sk-proj-abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(secret("sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345"));
        assert!(secret("sk-abcdefghijklmnopqrstuvwxyz012345"));
        // A fine grained GitHub token.
        assert!(secret("github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz"));
        // A GitLab personal access token.
        assert!(secret("glpat-abcdefghijklmnopqrstuvwxyz0123456789"));
        // A Google API key.
        assert!(secret("AIzaSyA1234567890abcdefghijklmnopqrstuv"));
        // The keys are found in a longer text as well.
        assert!(secret("token: AIzaSyA1234567890abcdefghijklmnopqrstuv"));
        assert!(secret("export TOKEN=glpat-abcdefghijklmnopqrstuvwxyz0123"));
    }

    #[test]
    fn the_new_secret_matchers_respect_the_minimum_length() {
        assert!(!secret("sk-proj-short"));
        assert!(!secret("sk-0123456789012345678"));
        assert!(!secret("github_pat_short"));
        assert!(!secret("github_pat_0123456789012345678"));
        assert!(!secret("glpat-short"));
        assert!(!secret("glpat-0123456789012345678"));
        assert!(!secret("AIzaShort"));
        assert!(!secret("AIzaSyA1234567890abcdefghijklmnopqrstu"));
    }

    #[test]
    fn registry_ignores_a_fenced_example_inside_a_list_item() {
        let dir = temp_dir("registry-list-fence");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        let fence = concat!(
            "- ```markdown\n",
            "  | Skill | When to read | Entrypoint |\n",
            "  | --- | --- | --- |\n",
            "  | Fake | When faking | [fake](fake/SKILL.md) |\n",
            "  ```\n",
        );
        let table = concat!(
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Ghost | When ghosting | [ghost](ghost/SKILL.md) |\n",
        );
        write(
            &registry,
            &format!("# Skills\n\n{fence}\n{table}\nTrailing text.\n"),
        );

        let report = reconcile_registry(&dir, &registry, true).unwrap();

        // The fenced example is not a table; the real table is reconciled.
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].entrypoint, "ghost/SKILL.md");
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].entrypoint, "demo/SKILL.md");

        let after = fs::read_to_string(&registry).unwrap();
        // The fenced example keeps its bytes.
        assert!(after.contains(fence), "{after}");
        assert!(
            after.contains("| Demo | A demo skill. | [demo](demo/SKILL.md) |"),
            "{after}"
        );
        assert!(after.ends_with("\nTrailing text.\n"), "{after}");
    }

    #[test]
    fn a_fence_inside_a_block_quote_is_a_fence() {
        let dir = temp_dir("registry-quote-fence");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        let fence = concat!(
            "> ```markdown\n",
            "> | Skill | When to read | Entrypoint |\n",
            "> | --- | --- | --- |\n",
            "> | Fake | When faking | [fake](fake/SKILL.md) |\n",
            "> ```\n",
        );
        let table = concat!(
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Demo | When demoing | [demo](demo/SKILL.md) |\n",
        );
        write(&registry, &format!("# Skills\n\n{fence}\n{table}\n"));

        let report = reconcile_registry(&dir, &registry, false).unwrap();

        // The quoted header is not the registry table.
        assert!(!report.out_of_sync());
        assert_eq!(report.kept, vec!["demo/SKILL.md".to_string()]);
    }

    #[test]
    fn fence_prefix_strips_the_markers_with_their_space() {
        // The space behind a marker belongs to the marker, so the three
        // spaces of the indentation are free behind every marker.
        assert_eq!(strip_fence_prefix(">    ```"), Some("```"));
        assert_eq!(strip_fence_prefix("> ```"), Some("```"));
        assert_eq!(strip_fence_prefix(">```"), Some("```"));
        assert_eq!(strip_fence_prefix("-    ```"), Some("```"));
        assert_eq!(strip_fence_prefix("   > ```"), Some("```"));
        assert_eq!(strip_fence_prefix("> - ```"), Some("```"));
        // A deeper indentation is an indented code block.
        assert_eq!(strip_fence_prefix("    ```"), None);
        assert_eq!(strip_fence_prefix(">     ```"), None);
    }

    #[test]
    fn a_fence_is_only_closed_by_its_own_container() {
        let top = Fence::opening("```markdown").unwrap();
        // A list item holds another container, so its fence is content of the
        // top level fence and not a closing fence.
        assert!(!top.closes("- ```"));
        assert!(!top.closes("> ```"));
        assert!(top.closes("```"));
        assert!(top.closes("  ```"));

        let list = Fence::opening("- ```markdown").unwrap();
        // The fence of a list item is closed by a fence in the same item.
        assert!(list.closes("  ```"));
        assert!(!list.closes("```"));
        assert!(!list.closes("- ```"));

        let quote = Fence::opening("> ```markdown").unwrap();
        // The fence of a block quote is closed by a fence in the same quote.
        assert!(quote.closes("> ```"));
        assert!(quote.closes(">    ```"));
        assert!(!quote.closes("```"));
    }

    #[test]
    fn a_closing_fence_in_a_block_quote_is_found() {
        let dir = temp_dir("registry-quote-fence-closer");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        // The closing fence sits behind the block quote marker and three
        // spaces; it closes the quoted example.
        let fence = concat!(
            "> ```markdown\n",
            "> | Skill | When to read | Entrypoint |\n",
            "> | --- | --- | --- |\n",
            "> | Fake | When faking | [fake](fake/SKILL.md) |\n",
            ">    ```\n",
        );
        let table = concat!(
            "| Skill | When to read | Entrypoint |\n",
            "| --- | --- | --- |\n",
            "| Demo | When demoing | [demo](demo/SKILL.md) |\n",
        );
        write(&registry, &format!("# Skills\n\n{fence}\n{table}\n"));

        let report = reconcile_registry(&dir, &registry, false).unwrap();

        // The quoted example is fenced, so the table below it is the
        // registry table.
        assert!(!report.out_of_sync());
        assert_eq!(report.exit_code(false), 0);
        assert_eq!(report.kept, vec!["demo/SKILL.md".to_string()]);
    }

    #[test]
    fn a_backtick_line_with_a_backtick_is_not_a_fence() {
        let dir = temp_dir("registry-inline-code");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            concat!(
                "# Skills\n",
                "\n",
                "```code``` here\n",
                "\n",
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n",
                "| Demo | When demoing | [demo](demo/SKILL.md) |\n",
            ),
        );

        let report = reconcile_registry(&dir, &registry, false).unwrap();

        // The inline code line is not a fence, so the table below it is the
        // registry table.
        assert!(!report.out_of_sync());
        assert_eq!(report.exit_code(false), 0);
        assert_eq!(report.kept, vec!["demo/SKILL.md".to_string()]);
    }

    #[test]
    fn link_destinations_accept_both_forms() {
        assert_eq!(
            extract_link_path("[demo](demo/SKILL.md)").as_deref(),
            Some("demo/SKILL.md")
        );
        assert_eq!(
            extract_link_path("[demo](<demo/SKILL.md>)").as_deref(),
            Some("demo/SKILL.md")
        );
        // A plain destination may hold balanced parentheses.
        assert_eq!(
            extract_link_path("[demo](demo(v2)/SKILL.md)").as_deref(),
            Some("demo(v2)/SKILL.md")
        );
        assert_eq!(
            extract_link_path("[demo](<demo(v2)/SKILL.md>)").as_deref(),
            Some("demo(v2)/SKILL.md")
        );
        assert_eq!(extract_link_path("demo/SKILL.md"), None);
    }

    #[test]
    fn body_link_targets_read_back_as_the_whole_path() {
        // A destination between angle brackets drops the brackets.
        assert_eq!(
            markdown_link_targets("Read the [guide](<docs/guide.md>)."),
            vec!["docs/guide.md".to_string()]
        );
        // A destination may hold balanced parentheses.
        assert_eq!(
            markdown_link_targets("Read the [guide](docs/guide(v2).md)."),
            vec!["docs/guide(v2).md".to_string()]
        );
        // Every link of a text is found.
        assert_eq!(
            markdown_link_targets("[a](docs/a.md) and [b](<docs/b.md>)"),
            vec!["docs/a.md".to_string(), "docs/b.md".to_string()]
        );
    }

    #[test]
    fn registry_rows_wrap_a_path_with_parentheses() {
        let dir = temp_dir("registry-parentheses");
        write(
            &dir.join("demo(v2)/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            "| Skill | When to read | Entrypoint |\n| --- | --- | --- |\n",
        );

        reconcile_registry(&dir, &registry, true).unwrap();

        let after = fs::read_to_string(&registry).unwrap();
        assert!(after.contains("[demo(v2)](<demo(v2)/SKILL.md>)"), "{after}");

        // The written row reads back as the same entrypoint, so the table is
        // in sync and a dry run succeeds.
        let report = reconcile_registry(&dir, &registry, false).unwrap();
        assert!(!report.out_of_sync());
        assert_eq!(report.exit_code(false), 0);
        assert_eq!(report.kept, vec!["demo(v2)/SKILL.md".to_string()]);
    }

    #[test]
    fn search_finds_a_skill_by_a_chinese_term() {
        let dir = search_fixture("search-chinese");
        let hits = search_skills(&dir, &SearchOptions::new("模型")).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "ai-tools");
        assert_eq!(hits[0].kind, SkillKind::Folder);
        assert_eq!(hits[0].entrypoint, "ai-tools/SKILL.md");
        assert_eq!(hits[0].title.as_deref(), Some("AI Tools"));
        assert_eq!(hits[0].summary.as_deref(), Some("When you need a model"));
        // The description holds the term and the body holds the description.
        assert_eq!(hits[0].score, 3 + 1);
    }

    #[test]
    fn search_finds_a_flat_skill_by_its_body() {
        let dir = search_fixture("search-body");
        let hits = search_skills(&dir, &SearchOptions::new("rsync")).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "deploy-guide");
        assert_eq!(hits[0].kind, SkillKind::Flat);
        assert_eq!(hits[0].entrypoint, "deploy-guide.md");
        // The body of the skill is the only field that holds the term.
        assert_eq!(hits[0].score, 1);
    }

    #[test]
    fn search_needs_every_term() {
        let dir = search_fixture("search-and");
        let hits = search_skills(&dir, &SearchOptions::new("ai 模型")).unwrap();

        // Both terms hold for the same skill, so the skill matches.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "ai-tools");

        // A term that holds for no skill at all matches nothing.
        let hits = search_skills(&dir, &SearchOptions::new("ai nothing-matches")).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn search_ranks_an_id_match_above_a_body_match() {
        let dir = search_fixture("search-rank");
        let hits = search_skills(&dir, &SearchOptions::new("guide")).unwrap();

        assert_eq!(hits.len(), 2);
        // The id of the flat skill holds the term, the body of the folder
        // skill only mentions it.
        assert_eq!(hits[0].id, "deploy-guide");
        assert_eq!(hits[1].id, "secrets");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn search_filters_by_kind_and_by_scripts() {
        let dir = search_fixture("search-filters");

        let mut options = SearchOptions::new("guide");
        options.kind = Some(SkillKind::Flat);
        let hits = search_skills(&dir, &options).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "deploy-guide");

        let mut options = SearchOptions::new("guide");
        options.has_scripts = true;
        let hits = search_skills(&dir, &options).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "secrets");
    }

    #[test]
    fn search_keeps_the_limit() {
        let dir = search_fixture("search-limit");
        let mut options = SearchOptions::new("guide");
        options.limit = 1;
        let hits = search_skills(&dir, &options).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "deploy-guide");
    }

    #[test]
    fn search_renders_the_json_shape() {
        let dir = search_fixture("search-json");
        let hits = search_skills(&dir, &SearchOptions::new("模型")).unwrap();
        let rendered = render_search_json(&hits).unwrap();

        assert!(rendered.ends_with("]\n"), "{rendered}");
        let parsed: Value = serde_json::from_str(&rendered).unwrap();
        let entries = parsed.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], "ai-tools");
        assert_eq!(entries[0]["kind"], "folder");
        assert_eq!(entries[0]["entrypoint"], "ai-tools/SKILL.md");
        assert_eq!(entries[0]["score"], 4);
        assert_eq!(entries[0]["title"], "AI Tools");
        assert_eq!(entries[0]["summary"], "When you need a model");
    }

    #[test]
    fn search_json_holds_null_for_a_missing_title_and_summary() {
        let dir = temp_dir("search-null");
        write(&dir.join("bare.md"), "# Bare\n\nA bare skill.\n");

        let hits = search_skills(&dir, &SearchOptions::new("bare")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, None);
        assert_eq!(hits[0].summary, None);

        let rendered = render_search_json(&hits).unwrap();
        let parsed: Value = serde_json::from_str(&rendered).unwrap();
        assert!(parsed[0]["title"].is_null());
        assert!(parsed[0]["summary"].is_null());
    }

    #[test]
    fn search_is_deterministic() {
        let dir = search_fixture("search-deterministic");

        let first = search_skills(&dir, &SearchOptions::new("guide")).unwrap();
        let second = search_skills(&dir, &SearchOptions::new("guide")).unwrap();
        assert_eq!(first, second);
        assert_eq!(render_search_text(&first), render_search_text(&second));
        assert!(render_search_text(&first).starts_with("deploy-guide  flat  deploy-guide.md"));
    }

    #[test]
    fn search_without_a_match_renders_the_empty_result() {
        let dir = search_fixture("search-empty");
        let hits = search_skills(&dir, &SearchOptions::new("nothing-matches")).unwrap();

        assert!(hits.is_empty());
        assert_eq!(render_search_text(&hits), "no skills match\n");
        assert_eq!(render_search_json(&hits).unwrap(), "[]\n");
    }

    #[test]
    fn search_without_a_registry_table_uses_the_frontmatter() {
        let dir = temp_dir("search-no-table");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        write(
            &dir.join(REGISTRY_FILE),
            "# Skills\n\nThe registry table is missing.\n",
        );

        let hits = search_skills(&dir, &SearchOptions::new("demo")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title.as_deref(), Some("Demo"));
        assert_eq!(hits[0].summary.as_deref(), Some("A demo skill."));
    }

    #[test]
    fn search_hides_a_secret_in_a_description() {
        let dir = temp_dir("search-secret");
        write(
            &dir.join("demo/SKILL.md"),
            concat!(
                "---\n",
                "name: Demo\n",
                "description: Uses AKIAIOSFODNN7EXAMPLE to sign the request.\n",
                "---\n",
                "\n",
                "# Demo\n",
            ),
        );

        let hits = search_skills(&dir, &SearchOptions::new("sign")).unwrap();
        assert_eq!(hits.len(), 1);

        let text = render_search_text(&hits);
        assert!(!text.contains("AKIAIOSFODNN7EXAMPLE"), "{text}");
        assert!(text.contains("<redacted>"), "{text}");

        let json = render_search_json(&hits).unwrap();
        assert!(!json.contains("AKIAIOSFODNN7EXAMPLE"), "{json}");
        assert!(json.contains("<redacted>"), "{json}");
    }

    #[test]
    fn search_hides_a_secret_that_crosses_the_cut_of_a_description() {
        let dir = temp_dir("search-secret-cut");
        // The key starts at the 110th character of the description, so the
        // cut of the summary would end inside the key.
        let description = format!("{} {SECRET}", "a".repeat(109));
        write(
            &dir.join("demo/SKILL.md"),
            &format!("---\nname: Demo\ndescription: {description}\n---\n\n# Demo\n"),
        );

        let hits = search_skills(&dir, &SearchOptions::new("demo")).unwrap();
        assert_eq!(hits.len(), 1);

        let summary = hits[0].summary.as_deref().expect("summary");
        assert!(!summary.contains(SECRET), "{summary}");
        assert!(!holds_piece_of(summary, SECRET), "{summary}");

        let text = render_search_text(&hits);
        assert!(!text.contains(SECRET), "{text}");
        assert!(!holds_piece_of(&text, SECRET), "{text}");
        assert!(text.contains("<redacted>"), "{text}");

        let json = render_search_json(&hits).unwrap();
        assert!(!json.contains(SECRET), "{json}");
        assert!(!holds_piece_of(&json, SECRET), "{json}");
        assert!(json.contains("<redacted>"), "{json}");
    }

    #[test]
    fn first_sentence_ends_at_the_earliest_terminator() {
        assert_eq!(first_sentence("Ready? Run this next.", 120), "Ready?");
        assert_eq!(first_sentence("Ready! Run this next.", 120), "Ready!");
        assert_eq!(first_sentence("准备好了！继续使用。", 120), "准备好了！");
        assert_eq!(first_sentence("准备好了？继续使用。", 120), "准备好了？");
        assert_eq!(
            first_sentence("First sentence. Second sentence.", 120),
            "First sentence"
        );
        assert_eq!(first_sentence("准备好了。继续使用。", 120), "准备好了");
        assert_eq!(first_sentence("First line\nSecond line", 120), "First line");
    }

    #[test]
    fn a_registry_row_never_holds_a_part_of_a_secret() {
        let dir = temp_dir("registry-secret-cut");
        let description = format!("{} {SECRET}", "a".repeat(109));
        write(
            &dir.join("demo/SKILL.md"),
            &format!("---\nname: Demo\ndescription: {description}\n---\n\n# Demo\n"),
        );
        let registry = dir.join(REGISTRY_FILE);
        write(
            &registry,
            "# Skills\n\n| Skill | When to read | Entrypoint |\n| --- | --- | --- |\n",
        );

        reconcile_registry(&dir, &registry, true).unwrap();

        let after = fs::read_to_string(&registry).unwrap();
        assert!(!after.contains(SECRET), "{after}");
        assert!(!holds_piece_of(&after, SECRET), "{after}");
    }

    /// Whether the text holds a piece of eight characters of the secret.
    fn holds_piece_of(text: &str, secret: &str) -> bool {
        secret
            .as_bytes()
            .windows(8)
            .any(|piece| text.contains(std::str::from_utf8(piece).unwrap()))
    }

    /// A skills directory with a Chinese skill, a skill with a script, a flat
    /// skill and a registry with titles.
    fn search_fixture(name: &str) -> PathBuf {
        let root = temp_dir(name);
        write(
            &root.join("ai-tools/SKILL.md"),
            concat!(
                "---\n",
                "name: AI Tools\n",
                "description: 大模型工具箱，帮助选择模型。More text here.\n",
                "tags: [ai, model]\n",
                "---\n",
                "\n",
                "# AI Tools\n",
                "\n",
                "Use this skill to pick a model.\n",
            ),
        );
        write(
            &root.join("secrets/SKILL.md"),
            concat!(
                "---\n",
                "name: Secrets\n",
                "description: Keeps credentials safe.\n",
                "---\n",
                "\n",
                "# Secrets\n",
                "\n",
                "Follow the deploy guide to keep credentials safe.\n",
            ),
        );
        write(&root.join("secrets/scripts/tool.py"), "print(\"hello\")\n");
        write(
            &root.join("deploy-guide.md"),
            "# Deploy guide\n\nCopy files with rsync to the host.\n",
        );
        write(
            &root.join(REGISTRY_FILE),
            concat!(
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n",
                "| AI Tools | When you need a model | [ai-tools](ai-tools/SKILL.md) |\n",
                "| Secrets | When you handle keys | [secrets](secrets/SKILL.md) |\n",
                "| Deploy Guide | When you deploy | [deploy-guide](deploy-guide.md) |\n",
            ),
        );
        root
    }

    /// The text of a line without the markers that may precede a fence.
    fn strip_fence_prefix(text: &str) -> Option<&str> {
        parse_fence_prefix(text).map(|prefix| prefix.rest)
    }

    fn fixture(name: &str) -> PathBuf {
        let root = temp_dir(name);
        write(
            &root.join("demo/SKILL.md"),
            concat!(
                "---\n",
                "name: Demo Skill\n",
                "description: Demo helper. More detail here.\n",
                "version: 1.2.3\n",
                "tags: [alpha, beta]\n",
                "requires: demo-docs\n",
                "---\n",
                "\n",
                "# Demo\n",
                "\n",
                "Read the [guide](docs/guide.md) or the [missing page](docs/missing.md).\n"
            ),
        );
        write(&root.join("demo/docs/guide.md"), "# Guide\n");
        write(
            &root.join("demo/scripts/tool.py"),
            "import json\nimport argparse\n\ndef main():\n    return json.dumps({})\n",
        );
        write(
            &root.join("bare/SKILL.md"),
            "# Bare\n\nNo frontmatter here.\n",
        );
        write(&root.join("howto.md"), "# Howto\n\nA flat skill.\n");
        write(
            &root.join(REGISTRY_FILE),
            concat!(
                "# Skills\n",
                "\n",
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n",
                "| Demo | When demoing | [demo](demo/SKILL.md) |\n",
                "| Ghost | When ghosting | [ghost](ghost/SKILL.md) |\n",
                "| Demo again | When demoing again | [demo](demo/SKILL.md) |\n",
                "\n",
                "Trailing text.\n"
            ),
        );
        root
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

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
            "skrun-catalog-{name}-{}-{now}-{count}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
