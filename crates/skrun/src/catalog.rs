//! # skrun
//!
//! Catalog owns repository level bookkeeping for guidance skills.
//!
//! ## Owns
//! - skill discovery for folder and flat skills
//! - deterministic JSON catalog index
//! - `SKILL_REGISTRY.md` reconciliation
//! - catalog consistency checks
//!
//! ## Must Not
//! - execute skills
//! - mutate skill payloads
//! - create a registry table that is not there
//! - call git or the network
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
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("registry: {}\n", self.label));
        out.push_str(&format!(
            "status: {}\n",
            if self.out_of_sync() {
                "out of sync"
            } else {
                "in sync"
            }
        ));
        for row in &self.removed {
            out.push_str(&format!("stale row ({}): {}\n", row.reason, row.entrypoint));
        }
        for entrypoint in &self.duplicates {
            out.push_str(&format!("duplicate row: {}\n", entrypoint));
        }
        for row in &self.missing {
            out.push_str(&format!("missing row: {} ({})\n", row.id, row.entrypoint));
        }
        if self.updated {
            out.push_str(&format!("updated registry: {}\n", self.label));
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
    let entries =
        fs::read_dir(dir).with_context(|| format!("read skills directory `{}`", dir.display()))?;
    let mut skills = Vec::new();

    for entry in entries {
        let entry = entry.with_context(|| format!("read skills directory `{}`", dir.display()))?;
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
            .with_context(|| format!("create directory `{}`", parent.display()))?;
    }
    fs::write(path, text).with_context(|| format!("write index `{}`", path.display()))
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
            registry.display(),
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
        label: display_path(dir, registry),
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
                skill.id
            ))),
            Some(name) if name != &skill.id => findings.push(Finding::warning(format!(
                "skill `{}` has frontmatter name `{name}` that differs from its id",
                skill.id
            ))),
            Some(_) => {}
        }
        if skill.frontmatter.description.is_none() {
            findings.push(Finding::error(format!(
                "skill `{}` is missing frontmatter `description`",
                skill.id
            )));
        }
    }

    for skill in &catalog.skills {
        for link in &skill.links {
            if !link.exists {
                findings.push(Finding::error(format!(
                    "skill `{}` has a broken link `{}`",
                    skill.id, link.target
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
        let label = display_path(dir, &path);
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
        findings.push(Finding::error(format!(
            "possible secret in `{path}:{line}`"
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
    let entries =
        fs::read_dir(root).with_context(|| format!("read skill directory `{}`", root.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("read skill directory `{}`", root.display()))?;
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

fn markdown_link_targets(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut targets = Vec::new();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b']' && bytes[index + 1] == b'(' {
            let start = index + 2;
            if let Some(offset) = text[start..].find(')') {
                targets.push(text[start..start + offset].to_string());
                index = start + offset + 1;
                continue;
            }
        }
        index += 1;
    }
    targets
}

fn registry_row(skill: &Skill) -> String {
    let title = skill
        .frontmatter
        .name
        .clone()
        .unwrap_or_else(|| skill.id.clone());
    let summary = summary_of(skill.frontmatter.description.as_deref());
    format!(
        "| {} | {} | [{}]({}) |",
        title,
        summary,
        skill.id,
        path_to_string(&skill.entrypoint)
    )
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
    let mut summary = text[..cut].trim().replace('|', "/");
    if summary.chars().count() > SUMMARY_LIMIT {
        summary = summary.chars().take(SUMMARY_LIMIT).collect();
    }
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
        .with_context(|| format!("read skills directory `{}`", root.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("read skills directory `{}`", root.display()))?;
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

fn line_has_secret(line: &str) -> bool {
    has_private_key(line)
        || has_aws_key(line)
        || has_openai_key(line)
        || has_github_token(line)
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
        let mut header = None;
        for (index, line) in lines.iter().enumerate() {
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
                && next.text.starts_with('|')
                && is_separator(&next.text)
            {
                start += 1;
            }
            // Data rows end at the first line that does not start with `|`.
            let mut end = start;
            while end < lines.len() && lines[end].text.starts_with('|') {
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

/// Read the registry file; a missing file reads as empty content.
fn read_registry(registry: &Path) -> Result<String> {
    match fs::read_to_string(registry) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("read registry `{}`", registry.display())),
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
    !cells.is_empty()
        && cells
            .iter()
            .all(|cell| cell.len() >= 3 && cell.chars().all(|character| character == '-'))
}

fn parse_cells(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn extract_link_path(cell: &str) -> Option<String> {
    let start = cell.find("](")? + 2;
    let rest = &cell[start..];
    let end = rest.find(')')?;
    Some(rest[..end].trim().to_string())
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

    fs::write(registry, content).with_context(|| format!("write registry `{}`", registry.display()))
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
