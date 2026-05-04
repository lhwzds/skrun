//! # codocia
//!
//! Runtime owns executable skill artifacts.
//!
//! ## Owns
//! - executable skill artifact manifest
//! - rust binary skill scaffold, build, and run
//! - uv-backed Python skill scaffold, sync, and run
//! - stdio JSON skill protocol enforcement
//! - artifact path validation
//!
//! ## Must Not
//! - own agent planning
//! - parse @skill mentions
//! - render UI
//! - own durable chat or task state
//! - manage provider credentials
//!
//! ## Inputs
//! - artifact directory
//! - JSON input value
//! - local Cargo or uv executable
//!
//! ## Outputs
//! - artifact manifest
//! - JSON output object
//! - build and run diagnostics
//!
//! ## Depends On
//! - serde_json
//!
//! ## Used By
//! - Python package
//! - future CLI and server skill entrypoints
//!
//! ## Verify
//! - cargo test -p runtime

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const ARTIFACT_FILE: &str = "artifact.json";
pub const SKILL_FILE: &str = "SKILL.md";
pub const DEFAULT_SCHEMA_VERSION: u32 = 1;
pub const SKILLS_DIR_ENV: &str = "SKRUN_SKILLS_DIR";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Markdown,
    RustBinary,
    PythonUv,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactProtocol {
    pub transport: String,
    pub input: String,
    pub output: String,
}

impl Default for ArtifactProtocol {
    fn default() -> Self {
        Self {
            transport: "stdio-json".to_string(),
            input: "single-json-value".to_string(),
            output: "single-json-value".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactSchema {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactSource {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillArtifact {
    pub schema_version: u32,
    pub kind: ArtifactKind,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(default = "default_executable")]
    pub executable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default)]
    pub protocol: ArtifactProtocol,
    #[serde(default)]
    pub schema: ArtifactSchema,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ArtifactSource>,
}

impl SkillArtifact {
    pub fn rust_binary(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        let id = id.into();
        Self {
            schema_version: DEFAULT_SCHEMA_VERSION,
            kind: ArtifactKind::RustBinary,
            id: id.clone(),
            name: name.into(),
            version: version.into(),
            description: None,
            tags: None,
            suggested_tools: Vec::new(),
            content: None,
            source_ref: None,
            executable: true,
            entry: Some(format!("bin/release/{}", executable_file_name(&id))),
            protocol: ArtifactProtocol::default(),
            schema: ArtifactSchema {
                input: Some("schema/input.json".to_string()),
                output: Some("schema/output.json".to_string()),
            },
            source: None,
        }
    }

    pub fn python_uv(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: DEFAULT_SCHEMA_VERSION,
            kind: ArtifactKind::PythonUv,
            id: id.into(),
            name: name.into(),
            version: version.into(),
            description: None,
            tags: None,
            suggested_tools: Vec::new(),
            content: None,
            source_ref: None,
            executable: true,
            entry: Some("skill.py".to_string()),
            protocol: ArtifactProtocol::default(),
            schema: ArtifactSchema {
                input: Some("schema/input.json".to_string()),
                output: Some("schema/output.json".to_string()),
            },
            source: None,
        }
    }

    pub fn markdown(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: DEFAULT_SCHEMA_VERSION,
            kind: ArtifactKind::Markdown,
            id: id.into(),
            name: name.into(),
            version: version.into(),
            description: None,
            tags: None,
            suggested_tools: Vec::new(),
            content: Some(content.into()),
            source_ref: None,
            executable: false,
            entry: None,
            protocol: ArtifactProtocol::default(),
            schema: ArtifactSchema::default(),
            source: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != DEFAULT_SCHEMA_VERSION {
            bail!(
                "unsupported artifact schema version: {}",
                self.schema_version
            );
        }
        validate_id(&self.id)?;
        match self.kind {
            ArtifactKind::Markdown => {
                if self.executable {
                    bail!("markdown skills must not be marked executable");
                }
            }
            ArtifactKind::RustBinary | ArtifactKind::PythonUv => {
                if !self.executable {
                    bail!("executable skill artifacts must be marked executable");
                }
                let entry = self
                    .entry
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("entry is required for executable skills"))?;
                validate_relative_path(entry, "entry")?;
                if self.protocol.transport != "stdio-json" {
                    bail!("unsupported artifact protocol transport");
                }
                if let Some(input) = &self.schema.input {
                    validate_relative_path(input, "schema.input")?;
                }
                if let Some(output) = &self.schema.output {
                    validate_relative_path(output, "schema.output")?;
                }
            }
        }
        Ok(())
    }

    pub fn entry_path(&self, root: impl AsRef<Path>) -> Result<PathBuf> {
        let entry = self
            .entry
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("entry is required for executable skills"))?;
        Ok(root.as_ref().join(entry))
    }
}

fn default_executable() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldOptions {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: ArtifactKind,
}

impl ScaffoldOptions {
    pub fn rust_binary(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            kind: ArtifactKind::RustBinary,
        }
    }

    pub fn python_uv(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            kind: ArtifactKind::PythonUv,
        }
    }

    pub fn markdown(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            kind: ArtifactKind::Markdown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    pub cargo: OsString,
    pub uv: OsString,
    pub profile: String,
    pub target_dir: Option<PathBuf>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            cargo: OsString::from("cargo"),
            uv: OsString::from("uv"),
            profile: "release".to_string(),
            target_dir: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOptions {
    pub uv: OsString,
    pub timeout: Duration,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            uv: OsString::from("uv"),
            timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRunOutput {
    pub value: Value,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOptions {
    pub root: PathBuf,
    pub skill_id: Option<String>,
    pub overwrite: bool,
}

impl InstallOptions {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            skill_id: None,
            overwrite: false,
        }
    }

    pub fn with_skill_id(mut self, skill_id: impl Into<String>) -> Self {
        self.skill_id = Some(skill_id.into());
        self
    }

    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }
}

pub fn artifact_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(ARTIFACT_FILE)
}

pub fn skill_markdown_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(SKILL_FILE)
}

#[derive(Debug, Default)]
struct MarkdownMetadata {
    name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
    suggested_tools: Vec<String>,
    version: Option<String>,
    source_ref: Option<String>,
    content: String,
}

fn load_markdown_artifact(root: &Path) -> Result<SkillArtifact> {
    let metadata = read_skill_markdown(root)?.ok_or_else(|| {
        anyhow::anyhow!(
            "skill directory must contain either {} or {}",
            ARTIFACT_FILE,
            SKILL_FILE
        )
    })?;
    let id = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("skill directory has no valid id"))?
        .to_string();
    validate_id(&id)?;
    let name = metadata
        .name
        .clone()
        .or_else(|| heading_title(&metadata.content))
        .unwrap_or_else(|| title_from_id(&id));
    let mut artifact = SkillArtifact::markdown(
        id,
        name,
        metadata
            .version
            .clone()
            .unwrap_or_else(|| "0.1.0".to_string()),
        metadata.content,
    );
    artifact.description = metadata.description;
    artifact.tags = metadata.tags;
    artifact.suggested_tools = metadata.suggested_tools;
    artifact.source_ref = metadata.source_ref;
    Ok(artifact)
}

fn apply_skill_markdown(root: &Path, artifact: &mut SkillArtifact) -> Result<()> {
    let Some(metadata) = read_skill_markdown(root)? else {
        return Ok(());
    };
    if artifact.description.is_none() {
        artifact.description = metadata.description;
    }
    if artifact.tags.is_none() {
        artifact.tags = metadata.tags;
    }
    if artifact.suggested_tools.is_empty() {
        artifact.suggested_tools = metadata.suggested_tools;
    }
    if artifact.content.is_none() {
        artifact.content = Some(metadata.content);
    }
    if artifact.source_ref.is_none() {
        artifact.source_ref = metadata.source_ref;
    }
    Ok(())
}

fn read_skill_markdown(root: &Path) -> Result<Option<MarkdownMetadata>> {
    let path = skill_markdown_path(root);
    if !path.is_file() {
        return Ok(None);
    }
    let markdown = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(Some(parse_skill_markdown(&markdown)))
}

fn parse_skill_markdown(markdown: &str) -> MarkdownMetadata {
    let mut metadata = MarkdownMetadata::default();
    if !markdown.starts_with("---") {
        metadata.content = markdown.trim().to_string();
        return metadata;
    }

    let lines = markdown.lines().collect::<Vec<_>>();
    let Some(end_line) = lines
        .iter()
        .skip(1)
        .position(|line| line.trim() == "---")
        .map(|index| index + 1)
    else {
        metadata.content = markdown.trim().to_string();
        return metadata;
    };

    parse_frontmatter(
        &lines[1..end_line].join(
            "
",
        ),
        &mut metadata,
    );
    metadata.content = lines[end_line + 1..]
        .join(
            "
",
        )
        .trim()
        .to_string();
    metadata
}

fn parse_frontmatter(frontmatter: &str, metadata: &mut MarkdownMetadata) {
    let mut current_list: Option<&str> = None;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            match current_list {
                Some("tags") => metadata
                    .tags
                    .get_or_insert_with(Vec::new)
                    .push(clean_scalar(item)),
                Some("suggested_tools") => metadata.suggested_tools.push(clean_scalar(item)),
                _ => {}
            }
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            current_list = None;
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        current_list = None;
        match key {
            "name" if !value.is_empty() => metadata.name = Some(clean_scalar(value)),
            "description" if !value.is_empty() => metadata.description = Some(clean_scalar(value)),
            "version" if !value.is_empty() => metadata.version = Some(clean_scalar(value)),
            "source_ref" | "ref" if !value.is_empty() => {
                metadata.source_ref = Some(clean_scalar(value));
            }
            "tags" => {
                if value.is_empty() {
                    metadata.tags = Some(Vec::new());
                    current_list = Some("tags");
                } else {
                    metadata.tags = Some(parse_inline_list(value));
                }
            }
            "suggested_tools" => {
                if value.is_empty() {
                    current_list = Some("suggested_tools");
                } else {
                    metadata.suggested_tools = parse_inline_list(value);
                }
            }
            _ => {}
        }
    }
}

fn parse_inline_list(value: &str) -> Vec<String> {
    let inner = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    inner
        .split(',')
        .map(clean_scalar)
        .filter(|item| !item.is_empty())
        .collect()
}

fn clean_scalar(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn heading_title(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let title = line.trim().strip_prefix("# ")?.trim();
        (!title.is_empty()).then(|| title.to_string())
    })
}

fn title_from_id(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn kind_label(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Markdown => "markdown",
        ArtifactKind::RustBinary => "rust_binary",
        ArtifactKind::PythonUv => "python_uv",
    }
}

pub fn load_artifact(root: impl AsRef<Path>) -> Result<SkillArtifact> {
    let root = root.as_ref();
    let artifact = if artifact_path(root).is_file() {
        let mut artifact: SkillArtifact = serde_json::from_str(
            &fs::read_to_string(artifact_path(root))
                .with_context(|| format!("read {}", artifact_path(root).display()))?,
        )
        .with_context(|| format!("decode {}", artifact_path(root).display()))?;
        apply_skill_markdown(root, &mut artifact)?;
        artifact
    } else {
        load_markdown_artifact(root)?
    };
    artifact.validate()?;
    Ok(artifact)
}

pub fn save_artifact(root: impl AsRef<Path>, artifact: &SkillArtifact) -> Result<()> {
    artifact.validate()?;
    let root = root.as_ref();
    fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    fs::write(
        artifact_path(root),
        serde_json::to_string_pretty(artifact)?.as_bytes(),
    )
    .with_context(|| format!("write {}", artifact_path(root).display()))?;
    Ok(())
}

pub fn scaffold_skill(root: impl AsRef<Path>, options: ScaffoldOptions) -> Result<SkillArtifact> {
    validate_id(&options.id)?;
    let root = root.as_ref();
    fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;

    let artifact = match options.kind {
        ArtifactKind::Markdown => {
            let content = format!(
                "# {}\n\nUse this guidance skill for focused agent instructions.",
                options.name
            );
            SkillArtifact::markdown(options.id, options.name, options.version, content)
        }
        ArtifactKind::RustBinary => {
            fs::create_dir_all(root.join("schema"))
                .with_context(|| format!("create {}", root.join("schema").display()))?;
            scaffold_rust_binary(root, &options)?;
            let artifact = SkillArtifact::rust_binary(options.id, options.name, options.version);
            write_default_schemas(root)?;
            artifact
        }
        ArtifactKind::PythonUv => {
            fs::create_dir_all(root.join("schema"))
                .with_context(|| format!("create {}", root.join("schema").display()))?;
            scaffold_python_uv(root, &options)?;
            let artifact = SkillArtifact::python_uv(options.id, options.name, options.version);
            write_default_schemas(root)?;
            artifact
        }
    };

    write_skill_markdown(root, &artifact)?;
    if artifact.executable {
        save_artifact(root, &artifact)?;
    }
    Ok(artifact)
}

pub fn build_skill(root: impl AsRef<Path>, options: &BuildOptions) -> Result<SkillArtifact> {
    let root = root.as_ref();
    let artifact = load_artifact(root)?;
    match artifact.kind {
        ArtifactKind::Markdown => bail!(
            "skill '{}' is guidance-only and cannot be built",
            artifact.id
        ),
        ArtifactKind::RustBinary => build_rust_binary(root, &artifact, options)?,
        ArtifactKind::PythonUv => build_python_uv(root, options)?,
    }
    Ok(artifact)
}

pub fn run_skill(
    root: impl AsRef<Path>,
    input: Value,
    options: &RunOptions,
) -> Result<SkillRunOutput> {
    let root = root
        .as_ref()
        .canonicalize()
        .with_context(|| format!("resolve {}", root.as_ref().display()))?;
    let artifact = load_artifact(&root)?;
    let mut command =
        match artifact.kind {
            ArtifactKind::Markdown => {
                bail!("skill '{}' is guidance-only and cannot be run", artifact.id)
            }
            ArtifactKind::RustBinary => {
                let executable = artifact.entry_path(&root)?;
                if !executable.is_file() {
                    bail!("skill executable not found: {}", executable.display());
                }
                let mut command = Command::new(executable);
                command.current_dir(&root);
                command
            }
            ArtifactKind::PythonUv => {
                let mut command = Command::new(&options.uv);
                command
                    .arg("run")
                    .arg("--project")
                    .arg(&root)
                    .arg("python")
                    .arg(artifact.entry.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("entry is required for python_uv skills")
                    })?);
                command.current_dir(&root);
                command
            }
        };

    let output = run_json_command(&mut command, &input, options.timeout)
        .with_context(|| format!("run skill `{}`", artifact.id))?;
    decode_skill_output(output)
}

pub fn default_skills_dir() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os(SKILLS_DIR_ENV) {
        return Ok(PathBuf::from(root));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".skrun").join("skills"))
}

pub fn list_installed_skills(root: impl AsRef<Path>) -> Result<Vec<SkillArtifact>> {
    let root = root.as_ref();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut artifacts = Vec::new();
    for item in fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
        let path = item?.path();
        if !path.is_dir()
            || (!artifact_path(&path).is_file() && !skill_markdown_path(&path).is_file())
        {
            continue;
        }
        if let Ok(artifact) = load_artifact(&path) {
            artifacts.push(artifact);
        }
    }
    artifacts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(artifacts)
}

pub fn install_local_skill(
    source: impl AsRef<Path>,
    options: &InstallOptions,
) -> Result<SkillArtifact> {
    let source = source.as_ref();
    let artifact = load_artifact(source)?;
    let skill_id = options.skill_id.as_deref().unwrap_or(&artifact.id);
    validate_id(skill_id)?;
    fs::create_dir_all(&options.root)
        .with_context(|| format!("create {}", options.root.display()))?;
    let target = options.root.join(skill_id);

    if target.exists() {
        if !options.overwrite {
            bail!("installed skill already exists: {}", target.display());
        }
        if same_path(source, &target)? {
            return Ok(artifact);
        }
        fs::remove_dir_all(&target).with_context(|| format!("remove {}", target.display()))?;
    }

    copy_dir_all(source, &target)
        .with_context(|| format!("install skill into {}", target.display()))?;
    load_artifact(&target)
}

pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("artifact id cannot be empty");
    }
    if !id
        .chars()
        .all(|item| item.is_ascii_alphanumeric() || item == '-' || item == '_')
    {
        bail!("artifact id must contain only ASCII letters, numbers, '-' or '_'");
    }
    if !id
        .chars()
        .next()
        .is_some_and(|item| item.is_ascii_alphanumeric())
    {
        bail!("artifact id must start with an ASCII letter or number");
    }
    Ok(())
}

fn validate_relative_path(path: &str, field: &str) -> Result<()> {
    if path.is_empty() {
        bail!("{field} cannot be empty");
    }
    let value = Path::new(path);
    if value.is_absolute() {
        bail!("{field} must be relative");
    }
    for component in value.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("{field} must not escape the artifact directory")
            }
        }
    }
    Ok(())
}

fn scaffold_rust_binary(root: &Path, options: &ScaffoldOptions) -> Result<()> {
    fs::create_dir_all(root.join("src"))
        .with_context(|| format!("create {}", root.join("src").display()))?;
    fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{id}"
version = "{version}"
edition = "2024"

[workspace]

[dependencies]
"#,
            id = options.id,
            version = options.version
        ),
    )
    .with_context(|| format!("write {}", root.join("Cargo.toml").display()))?;
    fs::write(
        root.join("src/main.rs"),
        r#"use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("read stdin");
    let output = if input.trim().is_empty() {
        "{}"
    } else {
        input.trim()
    };
    println!("{output}");
}
"#,
    )
    .with_context(|| format!("write {}", root.join("src/main.rs").display()))?;
    Ok(())
}

fn scaffold_python_uv(root: &Path, options: &ScaffoldOptions) -> Result<()> {
    fs::write(
        root.join("pyproject.toml"),
        format!(
            r#"[project]
name = "{id}"
version = "{version}"
requires-python = ">=3.11"
dependencies = []
"#,
            id = options.id,
            version = options.version
        ),
    )
    .with_context(|| format!("write {}", root.join("pyproject.toml").display()))?;
    fs::write(
        root.join("skill.py"),
        r#"import json
import sys


def main() -> None:
    raw = sys.stdin.read().strip()
    value = json.loads(raw) if raw else {}
    print(json.dumps(value, separators=(",", ":")))


if __name__ == "__main__":
    main()
"#,
    )
    .with_context(|| format!("write {}", root.join("skill.py").display()))?;
    Ok(())
}

fn write_default_schemas(root: &Path) -> Result<()> {
    let schema = r#"{
  "type": "object",
  "additionalProperties": true
}
"#;
    fs::write(root.join("schema/input.json"), schema)
        .with_context(|| format!("write {}", root.join("schema/input.json").display()))?;
    fs::write(root.join("schema/output.json"), schema)
        .with_context(|| format!("write {}", root.join("schema/output.json").display()))?;
    Ok(())
}

fn write_skill_markdown(root: &Path, artifact: &SkillArtifact) -> Result<()> {
    let description = artifact
        .description
        .as_deref()
        .unwrap_or(if artifact.executable {
            "Executable skrun skill."
        } else {
            "Guidance-only skrun skill."
        });
    let body = artifact.content.clone().unwrap_or_else(|| {
        format!(
            "# {}\n\n{}\n\n- id: `{}`\n- kind: `{}`\n- version: `{}`",
            artifact.name,
            description,
            artifact.id,
            kind_label(&artifact.kind),
            artifact.version
        )
    });
    fs::write(
        skill_markdown_path(root),
        format!(
            "---\nname: {}\ndescription: {}\nversion: {}\n---\n\n{}\n",
            artifact.name, description, artifact.version, body
        ),
    )
    .with_context(|| format!("write {}", skill_markdown_path(root).display()))?;
    Ok(())
}

fn build_rust_binary(root: &Path, artifact: &SkillArtifact, options: &BuildOptions) -> Result<()> {
    let target_dir = options.target_dir.clone().unwrap_or_else(|| {
        std::env::temp_dir()
            .join("skrun-skill-targets")
            .join(&artifact.id)
    });

    let mut command = Command::new(&options.cargo);
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir);
    if options.profile == "release" {
        command.arg("--release");
    } else {
        command.arg("--profile").arg(&options.profile);
    }
    run_status_command(&mut command, "build rust binary skill")?;

    let built_binary = target_dir
        .join(&options.profile)
        .join(executable_file_name(&artifact.id));
    let entry_path = artifact.entry_path(root)?;
    let entry_parent = entry_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("artifact entry has no parent"))?;
    fs::create_dir_all(entry_parent)
        .with_context(|| format!("create {}", entry_parent.display()))?;
    fs::copy(&built_binary, &entry_path).with_context(|| {
        format!(
            "copy built skill binary from {} to {}",
            built_binary.display(),
            entry_path.display()
        )
    })?;
    Ok(())
}

fn build_python_uv(root: &Path, options: &BuildOptions) -> Result<()> {
    if !root.join("uv.lock").exists() {
        let mut lock = Command::new(&options.uv);
        lock.arg("lock").arg("--project").arg(root);
        run_status_command(&mut lock, "lock python uv skill")?;
    }
    let mut sync = Command::new(&options.uv);
    sync.arg("sync").arg("--project").arg(root).arg("--locked");
    run_status_command(&mut sync, "sync python uv skill")?;
    Ok(())
}

fn run_json_command(command: &mut Command, input: &Value, timeout: Duration) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn skill process")?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("skill stdin was not piped"))?;
    serde_json::to_writer(&mut stdin, input).context("write skill input JSON")?;
    stdin
        .write_all(b"\n")
        .context("write skill input newline")?;
    drop(stdin);

    wait_with_timeout(child, timeout)
}

fn wait_with_timeout(mut child: std::process::Child, timeout: Duration) -> Result<Output> {
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().context("collect skill output");
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("skill process timed out after {}s", timeout.as_secs());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn decode_skill_output(output: Output) -> Result<SkillRunOutput> {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        bail!(
            "skill process exited with status {:?}: {}",
            output.status.code(),
            stderr
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: Value = serde_json::from_str(stdout.trim()).with_context(|| {
        format!(
            "decode skill output JSON from stdout: {}",
            stdout.trim().chars().take(120).collect::<String>()
        )
    })?;
    if !value.is_object() {
        bail!("skill output must be a JSON object");
    }
    Ok(SkillRunOutput {
        value,
        stderr,
        exit_code: output.status.code(),
    })
}

fn run_status_command(command: &mut Command, label: &str) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("{label}: spawn command"))?;
    if !output.status.success() {
        bail!(
            "{label} failed with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn executable_file_name(id: &str) -> String {
    format!("{id}{}", std::env::consts::EXE_SUFFIX)
}

fn same_path(left: &Path, right: &Path) -> Result<bool> {
    if !left.exists() || !right.exists() {
        return Ok(false);
    }
    Ok(fs::canonicalize(left)? == fs::canonicalize(right)?)
}

fn copy_dir_all(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target).with_context(|| format!("create {}", target.display()))?;
    for item in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let item = item?;
        let path = item.path();
        let target_path = target.join(item.file_name());
        if path.is_dir() {
            copy_dir_all(&path, &target_path)?;
        } else {
            fs::copy(&path, &target_path)
                .with_context(|| format!("copy {} to {}", path.display(), target_path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn artifact_rejects_unsafe_paths() {
        let mut artifact = SkillArtifact::rust_binary("safe-id", "Safe", "0.1.0");
        artifact.entry = Some("../bin/safe-id".to_string());

        let error = artifact.validate().unwrap_err();

        assert!(error.to_string().contains("entry must not escape"));
    }

    #[test]
    fn scaffold_rust_binary_writes_artifact_layout() {
        let root = temp_dir("rust-scaffold");
        let artifact = scaffold_skill(
            &root,
            ScaffoldOptions::rust_binary("echo-skill", "Echo Skill", "0.1.0"),
        )
        .unwrap();

        assert_eq!(artifact.kind, ArtifactKind::RustBinary);
        assert!(root.join("artifact.json").is_file());
        assert!(root.join("Cargo.toml").is_file());
        assert!(
            fs::read_to_string(root.join("Cargo.toml"))
                .unwrap()
                .contains("[workspace]")
        );
        assert!(root.join("src/main.rs").is_file());
        assert!(root.join("schema/input.json").is_file());
        assert!(load_artifact(&root).is_ok());
    }

    #[test]
    fn scaffold_python_uv_writes_artifact_layout() {
        let root = temp_dir("python-scaffold");
        let artifact = scaffold_skill(
            &root,
            ScaffoldOptions::python_uv("py-echo", "Python Echo", "0.1.0"),
        )
        .unwrap();

        assert_eq!(artifact.kind, ArtifactKind::PythonUv);
        assert!(root.join("artifact.json").is_file());
        assert!(root.join("pyproject.toml").is_file());
        assert!(root.join("skill.py").is_file());
        assert!(load_artifact(&root).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn run_rust_binary_artifact_executes_entry() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_dir("run-artifact");
        let artifact = SkillArtifact::rust_binary("echo", "Echo", "0.1.0");
        save_artifact(&root, &artifact).unwrap();
        let entry = artifact.entry_path(&root).unwrap();
        fs::create_dir_all(entry.parent().unwrap()).unwrap();
        fs::write(&entry, "#!/bin/sh\ncat\n").unwrap();
        let mut permissions = fs::metadata(&entry).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&entry, permissions).unwrap();

        let output = run_skill(
            &root,
            serde_json::json!({ "message": "hello" }),
            &RunOptions::default(),
        )
        .unwrap();

        assert_eq!(output.value, serde_json::json!({ "message": "hello" }));
    }

    #[cfg(unix)]
    #[test]
    fn run_rust_binary_artifact_accepts_relative_root() {
        use std::os::unix::fs::PermissionsExt;

        let base = relative_temp_dir("relative-run-base");
        let root = base.join("skill");
        fs::create_dir_all(&root).unwrap();
        let artifact = SkillArtifact::rust_binary("echo", "Echo", "0.1.0");
        save_artifact(&root, &artifact).unwrap();
        let entry = artifact.entry_path(&root).unwrap();
        fs::create_dir_all(entry.parent().unwrap()).unwrap();
        fs::write(&entry, "#!/bin/sh\ncat\n").unwrap();
        let mut permissions = fs::metadata(&entry).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&entry, permissions).unwrap();

        let output = run_skill(
            &root,
            serde_json::json!({ "message": "relative" }),
            &RunOptions::default(),
        )
        .unwrap();

        assert_eq!(output.value, serde_json::json!({ "message": "relative" }));
    }

    #[test]
    fn install_local_skill_copies_artifact_directory() {
        let source = temp_dir("install-source");
        let target = temp_dir("install-target");
        scaffold_skill(
            &source,
            ScaffoldOptions::python_uv("py-echo", "Python Echo", "0.1.0"),
        )
        .unwrap();

        let artifact = install_local_skill(&source, &InstallOptions::new(&target)).unwrap();
        let listed = list_installed_skills(&target).unwrap();

        assert_eq!(artifact.id, "py-echo");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "py-echo");
        assert!(target.join("py-echo").join("artifact.json").is_file());
    }

    #[test]
    fn markdown_skill_loads_without_artifact_json() {
        let root = temp_dir("markdown-load");
        fs::write(
            root.join("SKILL.md"),
            "---\nname: Team\ndescription: Coordinate workers.\ntags:\n  - system\n  - team\nsuggested_tools:\n  - spawn_agent\nversion: 1.2.3\n---\n\n# Team\n\nUse parallel workers.",
        )
        .unwrap();

        let artifact = load_artifact(&root).unwrap();

        assert_eq!(artifact.kind, ArtifactKind::Markdown);
        assert_eq!(artifact.id, root.file_name().unwrap().to_str().unwrap());
        assert_eq!(artifact.name, "Team");
        assert_eq!(artifact.description.as_deref(), Some("Coordinate workers."));
        assert_eq!(
            artifact.tags,
            Some(vec!["system".to_string(), "team".to_string()])
        );
        assert_eq!(artifact.suggested_tools, vec!["spawn_agent"]);
        assert_eq!(artifact.version, "1.2.3");
        assert_eq!(
            artifact.content.as_deref(),
            Some("# Team\n\nUse parallel workers.")
        );
        assert!(!artifact.executable);
        assert!(artifact.entry.is_none());
    }

    #[test]
    fn markdown_skill_without_frontmatter_uses_heading() {
        let root = temp_dir("markdown-heading");
        fs::write(root.join("SKILL.md"), "# Review Code\n\nInspect changes.").unwrap();

        let artifact = load_artifact(&root).unwrap();

        assert_eq!(artifact.kind, ArtifactKind::Markdown);
        assert_eq!(artifact.name, "Review Code");
        assert_eq!(artifact.version, "0.1.0");
        assert_eq!(
            artifact.content.as_deref(),
            Some("# Review Code\n\nInspect changes.")
        );
    }

    #[test]
    fn markdown_skill_run_returns_guidance_only_error() {
        let root = temp_dir("markdown-run");
        fs::write(root.join("SKILL.md"), "# Plan\n\nThink first.").unwrap();

        let error = run_skill(&root, serde_json::json!({}), &RunOptions::default()).unwrap_err();

        assert!(error.to_string().contains("guidance-only"));
    }

    #[test]
    fn install_local_skill_copies_markdown_directory() {
        let source = temp_dir("markdown-install-source");
        let target = temp_dir("markdown-install-target");
        fs::write(
            source.join("SKILL.md"),
            "---\nname: Planner\n---\n\n# Planner\n",
        )
        .unwrap();

        let artifact = install_local_skill(&source, &InstallOptions::new(&target)).unwrap();
        let listed = list_installed_skills(&target).unwrap();

        assert_eq!(artifact.kind, ArtifactKind::Markdown);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].kind, ArtifactKind::Markdown);
        assert!(target.join(artifact.id).join("SKILL.md").is_file());
    }

    #[test]
    fn scaffold_markdown_writes_skill_only_and_round_trips_content() {
        let root = temp_dir("markdown-scaffold");

        let artifact = scaffold_skill(
            &root,
            ScaffoldOptions::markdown("plan-first", "Plan First", "0.2.0"),
        )
        .unwrap();
        let loaded = load_artifact(&root).unwrap();

        assert_eq!(artifact.kind, ArtifactKind::Markdown);
        assert!(!root.join("artifact.json").exists());
        assert!(root.join("SKILL.md").is_file());
        assert_eq!(loaded.kind, ArtifactKind::Markdown);
        assert_eq!(loaded.content, artifact.content);
        assert_eq!(
            loaded.content.as_deref(),
            Some("# Plan First\n\nUse this guidance skill for focused agent instructions.")
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("skrun-runtime-{name}-{}-{now}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn relative_temp_dir(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let absolute_root =
            std::env::temp_dir().join(format!("skrun-runtime-{name}-{}-{now}", std::process::id()));
        fs::create_dir_all(&absolute_root).unwrap();

        let current_dir = std::env::current_dir().unwrap();
        let mut relative = PathBuf::new();
        for component in current_dir.components() {
            if matches!(component, std::path::Component::Normal(_)) {
                relative.push("..");
            }
        }
        for component in absolute_root.components() {
            if let std::path::Component::Normal(part) = component {
                relative.push(part);
            }
        }
        relative
    }
}
