//! # skrun
//!
//! Catalog owns repository level bookkeeping for guidance skills.
//!
//! ## Owns
//! - skill discovery for folder and flat skills
//! - deterministic JSON catalog index
//! - `SKILL_REGISTRY.md` reconciliation
//! - catalog consistency checks
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
//!
//! ## Depends On
//! - serde_json
//!
//! ## Verify
//! - cargo test -p skrun

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

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
pub fn render_index(index: &CatalogIndex) -> Result<String> {
    let mut text = serde_json::to_string_pretty(index)?;
    text.push('\n');
    Ok(text)
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

/// Parse the optional frontmatter block of a skill entrypoint.
///
/// The block only counts when its first line is exactly `---` and a later
/// line is exactly `---` as well. An unterminated block is not frontmatter.
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
            "name" => frontmatter.name = non_empty(value),
            "description" => frontmatter.description = non_empty(value),
            "version" => frontmatter.version = non_empty(value),
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

fn parse_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let inner = if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    inner
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\'').trim())
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
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

fn script_language(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("py") => Some("python"),
        Some("sh") => Some("shell"),
        Some("js") | Some("mjs") => Some("javascript"),
        Some("ts") => Some("typescript"),
        _ => None,
    }
}

fn python_imports(text: &str) -> Vec<String> {
    let mut imports = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let keyword = if trimmed.starts_with("import") {
            "import"
        } else if trimmed.starts_with("from") {
            "from"
        } else {
            continue;
        };
        let rest = &trimmed[keyword.len()..];
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let rest = rest.trim_start();
        let mut characters = rest.chars();
        let Some(first) = characters.next() else {
            continue;
        };
        if !first.is_ascii_alphabetic() && first != '_' {
            continue;
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
        if name == "__future__" {
            continue;
        }
        imports.insert(name);
    }
    imports.into_iter().collect()
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

fn summary_of(description: Option<&str>) -> String {
    let Some(text) = description else {
        return "(no description)".to_string();
    };
    let mut cut = text.len();
    if let Some(position) = text.find(". ") {
        cut = cut.min(position);
    }
    if let Some(position) = text.find('。') {
        cut = cut.min(position);
    }
    if let Some(position) = text.find('\n') {
        cut = cut.min(position);
    }
    let mut summary = text[..cut].trim().to_string();
    if summary.chars().count() > SUMMARY_LIMIT {
        summary = summary.chars().take(SUMMARY_LIMIT).collect();
    }
    let summary = escape_cell(&summary);
    if summary.is_empty() {
        "(no description)".to_string()
    } else {
        summary
    }
}

fn index_is_stale(path: &Path, catalog: &CatalogIndex) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return true;
    };
    let Ok(committed) = serde_json::from_str::<Value>(&text) else {
        return true;
    };
    let Ok(fresh) = serde_json::to_value(catalog) else {
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
        for (index, line) in text.lines().enumerate() {
            if line_has_secret(line) {
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
    if line_has_secret(text) {
        REDACTED.to_string()
    } else {
        text.to_string()
    }
}

fn line_has_secret(line: &str) -> bool {
    has_private_key(line)
        || has_aws_key(line)
        || has_openai_key(line)
        || has_sk_key(line)
        || has_github_token(line)
        || has_github_pat(line)
        || has_gitlab_token(line)
        || has_google_key(line)
        || has_slack_token(line)
}

fn has_private_key(line: &str) -> bool {
    let marker = "-----BEGIN ";
    let mut search = 0;
    while let Some(offset) = line[search..].find(marker) {
        let at = search + offset;
        let rest = &line[at + marker.len()..];
        if let Some(position) = rest.find("PRIVATE KEY-----")
            && rest[..position]
                .chars()
                .all(|character| character == ' ' || character.is_ascii_uppercase())
        {
            return true;
        }
        search = at + 1;
    }
    false
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
    use std::time::{SystemTime, UNIX_EPOCH};

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
        assert!(line_has_secret("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(line_has_secret("-----BEGIN PRIVATE KEY-----"));
        assert!(line_has_secret("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(line_has_secret(SECRET));
        assert!(line_has_secret("sk-abcdefghijklmnopqrstuvwxyz012345"));
        assert!(line_has_secret("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(line_has_secret("gho_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(line_has_secret("xoxb-1234567890-abcdefghij"));
        assert!(!line_has_secret("nothing to see here"));
        assert!(!line_has_secret("sk-short"));
        assert!(!line_has_secret("AKIASHORT"));
        assert!(!line_has_secret("-----BEGIN CERTIFICATE-----"));
        assert!(!line_has_secret("risk-analysis-notes"));
    }

    #[test]
    fn secret_matchers_respect_the_minimum_length() {
        assert!(line_has_secret("AKIA0123456789ABCDEF"));
        assert!(!line_has_secret("AKIA0123456789ABCDE"));
        assert!(line_has_secret("sk-012345678901234567890"));
        assert!(!line_has_secret("sk-0123456789012345678"));
        assert!(line_has_secret("ghp_012345678901234567890123456789"));
        assert!(!line_has_secret("ghp_01234567890123456789012345678"));
        assert!(line_has_secret("xoxb-0123456789"));
        assert!(!line_has_secret("xoxb-012345678"));
    }

    #[test]
    fn detects_the_new_secret_shapes() {
        // A project key, an application key and every other `sk-` key.
        assert!(line_has_secret(
            "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789"
        ));
        assert!(line_has_secret(
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345"
        ));
        assert!(line_has_secret("sk-abcdefghijklmnopqrstuvwxyz012345"));
        // A fine grained GitHub token.
        assert!(line_has_secret(
            "github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz"
        ));
        // A GitLab personal access token.
        assert!(line_has_secret(
            "glpat-abcdefghijklmnopqrstuvwxyz0123456789"
        ));
        // A Google API key.
        assert!(line_has_secret("AIzaSyA1234567890abcdefghijklmnopqrstuv"));
        // The keys are found in a longer text as well.
        assert!(line_has_secret(
            "token: AIzaSyA1234567890abcdefghijklmnopqrstuv"
        ));
        assert!(line_has_secret(
            "export TOKEN=glpat-abcdefghijklmnopqrstuvwxyz0123"
        ));
    }

    #[test]
    fn the_new_secret_matchers_respect_the_minimum_length() {
        assert!(!line_has_secret("sk-proj-short"));
        assert!(!line_has_secret("sk-0123456789012345678"));
        assert!(!line_has_secret("github_pat_short"));
        assert!(!line_has_secret("github_pat_0123456789012345678"));
        assert!(!line_has_secret("glpat-short"));
        assert!(!line_has_secret("glpat-0123456789012345678"));
        assert!(!line_has_secret("AIzaShort"));
        assert!(!line_has_secret("AIzaSyA1234567890abcdefghijklmnopqrstu"));
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
        let root =
            std::env::temp_dir().join(format!("skrun-catalog-{name}-{}-{now}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
