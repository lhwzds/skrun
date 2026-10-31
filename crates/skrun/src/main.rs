//! # skrun
//!
//! CLI owns the minimal executable skill command loop.
//!
//! ## Owns
//! - skill artifact scaffolding command
//! - skill artifact build command
//! - skill artifact run command
//! - local skill install and list commands
//! - catalog index, registry and check commands
//!
//! ## Must Not
//! - start the legacy daemon
//! - render TUI state
//! - own runtime skill execution
//! - own Python package behavior
//!
//! ## Inputs
//! - command line arguments
//! - JSON input strings
//! - local artifact directories
//!
//! ## Outputs
//! - human-readable command status
//! - JSON skill outputs
//!
//! ## Depends On
//! - skrun public runtime API
//!
//! ## Verify
//! - cargo test -p skrun

use anyhow::{Context, Result, bail};
use serde_json::Value;
use skrun::catalog;
use skrun::{
    ArtifactKind, BuildOptions, InstallOptions, RunOptions, ScaffoldOptions, build_skill,
    default_skills_dir, install_local_skill, list_installed_skills, load_artifact, run_skill,
    scaffold_skill,
};
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let command = parse(args)?;
    match command {
        Command::Help => {
            print_usage();
            Ok(())
        }
        Command::Skill(SkillCommand::New(options)) => {
            let artifact = scaffold_skill(&options.dir, options.scaffold)?;
            println!(
                "Created {} skill `{}` at {}",
                kind_label(&artifact.kind),
                artifact.id,
                options.dir.display()
            );
            Ok(())
        }
        Command::Skill(SkillCommand::Build(options)) => {
            let artifact = build_skill(&options.dir, &options.build)?;
            println!("Built skill `{}` at {}", artifact.id, options.dir.display());
            Ok(())
        }
        Command::Skill(SkillCommand::Run(options)) => {
            let root = match options.target {
                RunTarget::Dir(dir) => dir,
                RunTarget::Installed { root, id } => root.join(id),
            };
            let output = run_skill(root, options.input, &options.run)?;
            println!("{}", serde_json::to_string_pretty(&output.value)?);
            Ok(())
        }
        Command::Skill(SkillCommand::InstallLocal(options)) => {
            let artifact = install_local_skill(&options.source, &options.install)?;
            println!(
                "Installed skill `{}` into {}",
                artifact.id,
                options.install.root.display()
            );
            Ok(())
        }
        Command::Skill(SkillCommand::List(options)) => {
            let artifacts = list_installed_skills(&options.root)?;
            if options.format == OutputFormat::Json {
                println!("{}", serde_json::to_string_pretty(&artifacts)?);
            } else {
                for artifact in artifacts {
                    println!(
                        "{}\t{}\t{}\t{}",
                        artifact.id,
                        kind_label(&artifact.kind),
                        artifact.version,
                        artifact.name
                    );
                }
            }
            Ok(())
        }
        Command::Skill(SkillCommand::Show(options)) => {
            let artifact = load_artifact(options.root.join(&options.id))?;
            if options.format == OutputFormat::Json {
                println!("{}", serde_json::to_string_pretty(&artifact)?);
            } else {
                println!(
                    "{}\t{}\t{}\t{}",
                    artifact.id,
                    kind_label(&artifact.kind),
                    artifact.version,
                    artifact.name
                );
            }
            Ok(())
        }
        Command::Catalog(CatalogCommand::Index(options)) => {
            let index = catalog::build_index(&options.dir)?;
            if let Some(rendered) = catalog_index_output(&index, options.out.as_deref())? {
                print!("{rendered}");
            }
            Ok(())
        }
        Command::Catalog(CatalogCommand::Registry(options)) => {
            let code = run_catalog_registry(&options)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Command::Catalog(CatalogCommand::Check(options)) => {
            let report = catalog::check_catalog(
                &options.dir,
                options.registry.as_deref(),
                options.index.as_deref(),
            )?;
            print!("{}", report.render());
            if report.has_errors() {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Help,
    Skill(SkillCommand),
    Catalog(CatalogCommand),
}

#[derive(Debug, Clone, PartialEq)]
enum SkillCommand {
    New(NewCommand),
    Build(BuildCommand),
    Run(RunCommand),
    InstallLocal(InstallLocalCommand),
    List(ListCommand),
    Show(ShowCommand),
}

#[derive(Debug, Clone, PartialEq)]
enum CatalogCommand {
    Index(CatalogIndexCommand),
    Registry(CatalogRegistryCommand),
    Check(CatalogCheckCommand),
}

#[derive(Debug, Clone, PartialEq)]
struct NewCommand {
    dir: PathBuf,
    scaffold: ScaffoldOptions,
}

#[derive(Debug, Clone, PartialEq)]
struct BuildCommand {
    dir: PathBuf,
    build: BuildOptions,
}

#[derive(Debug, Clone, PartialEq)]
struct RunCommand {
    target: RunTarget,
    input: Value,
    run: RunOptions,
}

#[derive(Debug, Clone, PartialEq)]
enum RunTarget {
    Dir(PathBuf),
    Installed { root: PathBuf, id: String },
}

#[derive(Debug, Clone, PartialEq)]
struct InstallLocalCommand {
    source: PathBuf,
    install: InstallOptions,
}

#[derive(Debug, Clone, PartialEq)]
struct ListCommand {
    root: PathBuf,
    format: OutputFormat,
}

#[derive(Debug, Clone, PartialEq)]
struct ShowCommand {
    root: PathBuf,
    id: String,
    format: OutputFormat,
}

#[derive(Debug, Clone, PartialEq)]
struct CatalogIndexCommand {
    dir: PathBuf,
    out: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
struct CatalogRegistryCommand {
    dir: PathBuf,
    registry: Option<PathBuf>,
    write: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct CatalogCheckCommand {
    dir: PathBuf,
    registry: Option<PathBuf>,
    index: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

fn parse(args: Vec<String>) -> Result<Command> {
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help" | "help") {
        return Ok(Command::Help);
    }
    let mut cursor = Cursor::new(args);
    match cursor.next_required("command")?.as_str() {
        "skill" => parse_skill(cursor).map(Command::Skill),
        "catalog" => parse_catalog(cursor).map(Command::Catalog),
        other => bail!("unknown command `{other}`"),
    }
}

fn parse_skill(mut cursor: Cursor) -> Result<SkillCommand> {
    match cursor.next_required("skill command")?.as_str() {
        "new" => parse_skill_new(cursor).map(SkillCommand::New),
        "build" => parse_skill_build(cursor).map(SkillCommand::Build),
        "run" => parse_skill_run(cursor).map(SkillCommand::Run),
        "install-local" => parse_skill_install_local(cursor).map(SkillCommand::InstallLocal),
        "list" => parse_skill_list(cursor).map(SkillCommand::List),
        "show" => parse_skill_show(cursor).map(SkillCommand::Show),
        other => bail!("unknown skill command `{other}`"),
    }
}

fn parse_skill_new(mut cursor: Cursor) -> Result<NewCommand> {
    let mut kind = None;
    let mut id = None;
    let mut name = None;
    let mut version = "0.1.0".to_string();
    let mut dir = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--kind" => kind = Some(parse_kind(&cursor.next_required("--kind value")?)?),
            "--id" => id = Some(cursor.next_required("--id value")?),
            "--name" => name = Some(cursor.next_required("--name value")?),
            "--version" => version = cursor.next_required("--version value")?,
            value if value.starts_with('-') => bail!("unknown option `{value}`"),
            value => dir = Some(PathBuf::from(value)),
        }
    }

    let kind = kind.ok_or_else(|| anyhow::anyhow!("missing --kind"))?;
    let id = id.ok_or_else(|| anyhow::anyhow!("missing --id"))?;
    let name = name.unwrap_or_else(|| title_from_id(&id));
    let dir = dir.ok_or_else(|| anyhow::anyhow!("missing skill directory"))?;
    let scaffold = match kind {
        ArtifactKind::Markdown => ScaffoldOptions::markdown(id, name, version),
        ArtifactKind::RustBinary => ScaffoldOptions::rust_binary(id, name, version),
        ArtifactKind::PythonUv => ScaffoldOptions::python_uv(id, name, version),
    };

    Ok(NewCommand { dir, scaffold })
}

fn parse_skill_build(mut cursor: Cursor) -> Result<BuildCommand> {
    let mut build = BuildOptions::default();
    let mut dir = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--profile" => build.profile = cursor.next_required("--profile value")?,
            "--target-dir" => {
                build.target_dir = Some(PathBuf::from(cursor.next_required("--target-dir value")?))
            }
            "--cargo" => build.cargo = OsString::from(cursor.next_required("--cargo value")?),
            "--uv" => build.uv = OsString::from(cursor.next_required("--uv value")?),
            value if value.starts_with('-') => bail!("unknown option `{value}`"),
            value => dir = Some(PathBuf::from(value)),
        }
    }

    Ok(BuildCommand {
        dir: dir.ok_or_else(|| anyhow::anyhow!("missing skill directory"))?,
        build,
    })
}

fn parse_skill_run(mut cursor: Cursor) -> Result<RunCommand> {
    let mut run = RunOptions::default();
    let mut input = serde_json::json!({});
    let mut dir = None;
    let mut id = None;
    let mut root = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--input" => {
                input = serde_json::from_str(&cursor.next_required("--input value")?)
                    .context("parse --input JSON")?;
                if !input.is_object() {
                    bail!("--input must be a JSON object");
                }
            }
            "--timeout" => {
                let seconds = cursor
                    .next_required("--timeout value")?
                    .parse::<u64>()
                    .context("parse --timeout seconds")?;
                run.timeout = Duration::from_secs(seconds);
            }
            "--uv" => run.uv = OsString::from(cursor.next_required("--uv value")?),
            "--id" => id = Some(cursor.next_required("--id value")?),
            "--root" => root = Some(PathBuf::from(cursor.next_required("--root value")?)),
            value if value.starts_with('-') => bail!("unknown option `{value}`"),
            value => dir = Some(PathBuf::from(value)),
        }
    }

    let target = match (dir, id) {
        (Some(_), Some(_)) => bail!("skill run accepts either --id or a directory, not both"),
        (Some(dir), None) => RunTarget::Dir(dir),
        (None, Some(id)) => RunTarget::Installed {
            root: root.unwrap_or(default_skills_dir()?),
            id,
        },
        (None, None) => bail!("missing skill directory or --id"),
    };

    Ok(RunCommand { target, input, run })
}

fn parse_skill_install_local(mut cursor: Cursor) -> Result<InstallLocalCommand> {
    let mut root = None;
    let mut skill_id = None;
    let mut overwrite = false;
    let mut source = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(cursor.next_required("--root value")?)),
            "--id" => skill_id = Some(cursor.next_required("--id value")?),
            "--overwrite" => overwrite = true,
            value if value.starts_with('-') => bail!("unknown option `{value}`"),
            value => source = Some(PathBuf::from(value)),
        }
    }

    let root = root.unwrap_or(default_skills_dir()?);
    let mut install = InstallOptions::new(root).with_overwrite(overwrite);
    if let Some(skill_id) = skill_id {
        install = install.with_skill_id(skill_id);
    }

    Ok(InstallLocalCommand {
        source: source.ok_or_else(|| anyhow::anyhow!("missing source skill directory"))?,
        install,
    })
}

fn parse_skill_list(mut cursor: Cursor) -> Result<ListCommand> {
    let mut root = None;
    let mut format = OutputFormat::Text;
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(cursor.next_required("--root value")?)),
            "--format" => format = parse_output_format(&cursor.next_required("--format value")?)?,
            value => bail!("unknown argument `{value}`"),
        }
    }
    Ok(ListCommand {
        root: root.unwrap_or(default_skills_dir()?),
        format,
    })
}

fn parse_skill_show(mut cursor: Cursor) -> Result<ShowCommand> {
    let mut root = None;
    let mut format = OutputFormat::Text;
    let mut id = None;
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(cursor.next_required("--root value")?)),
            "--format" => format = parse_output_format(&cursor.next_required("--format value")?)?,
            value if value.starts_with('-') => bail!("unknown argument `{value}`"),
            value => id = Some(value.to_string()),
        }
    }
    Ok(ShowCommand {
        root: root.unwrap_or(default_skills_dir()?),
        id: id.ok_or_else(|| anyhow::anyhow!("missing skill id"))?,
        format,
    })
}

fn parse_catalog(mut cursor: Cursor) -> Result<CatalogCommand> {
    match cursor.next_required("catalog command")?.as_str() {
        "index" => parse_catalog_index(cursor).map(CatalogCommand::Index),
        "registry" => parse_catalog_registry(cursor).map(CatalogCommand::Registry),
        "check" => parse_catalog_check(cursor).map(CatalogCommand::Check),
        other => bail!("unknown catalog command `{other}`"),
    }
}

fn parse_catalog_index(mut cursor: Cursor) -> Result<CatalogIndexCommand> {
    let mut out = None;
    let mut dir = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(cursor.next_required("--out value")?)),
            value if value.starts_with('-') => bail!("unknown option `{value}`"),
            value => dir = Some(PathBuf::from(value)),
        }
    }

    Ok(CatalogIndexCommand {
        dir: dir.ok_or_else(|| anyhow::anyhow!("missing skills directory"))?,
        out,
    })
}

fn parse_catalog_registry(mut cursor: Cursor) -> Result<CatalogRegistryCommand> {
    let mut registry = None;
    let mut write = false;
    let mut dir = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--registry" => {
                registry = Some(PathBuf::from(cursor.next_required("--registry value")?))
            }
            "--write" => write = true,
            value if value.starts_with('-') => bail!("unknown option `{value}`"),
            value => dir = Some(PathBuf::from(value)),
        }
    }

    Ok(CatalogRegistryCommand {
        dir: dir.ok_or_else(|| anyhow::anyhow!("missing skills directory"))?,
        registry,
        write,
    })
}

fn parse_catalog_check(mut cursor: Cursor) -> Result<CatalogCheckCommand> {
    let mut registry = None;
    let mut index = None;
    let mut dir = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--registry" => {
                registry = Some(PathBuf::from(cursor.next_required("--registry value")?))
            }
            "--index" => index = Some(PathBuf::from(cursor.next_required("--index value")?)),
            value if value.starts_with('-') => bail!("unknown option `{value}`"),
            value => dir = Some(PathBuf::from(value)),
        }
    }

    Ok(CatalogCheckCommand {
        dir: dir.ok_or_else(|| anyhow::anyhow!("missing skills directory"))?,
        registry,
        index,
    })
}

/// Run the `catalog registry` command and return its exit status.
///
/// The report is printed as it is; the status follows the mode. A dry run
/// fails when the registry table drifted so a stale table is noticed. A write
/// run already repaired the table, so it succeeds and only a real error, such
/// as a registry file that cannot be read or written, fails the command.
///
/// A registry file without the registry table is a real error as well: the
/// command fails with a clear message and the file stays untouched.
fn run_catalog_registry(options: &CatalogRegistryCommand) -> Result<i32> {
    let registry = options
        .registry
        .clone()
        .unwrap_or_else(|| options.dir.join(catalog::REGISTRY_FILE));
    let report = catalog::reconcile_registry(&options.dir, &registry, options.write)?;
    print!("{}", report.render());
    Ok(report.exit_code(options.write))
}

/// Render the output of the `catalog index` command.
///
/// Without `--out` the rendered index is returned for printing. With `--out`
/// the index is written to the file and nothing is printed, so the given path
/// never shows up in the command output.
fn catalog_index_output(
    index: &catalog::CatalogIndex,
    out: Option<&Path>,
) -> Result<Option<String>> {
    let rendered = catalog::render_index(index)?;
    match out {
        Some(path) => {
            catalog::write_index_file(path, &rendered)?;
            Ok(None)
        }
        None => Ok(Some(rendered)),
    }
}

fn parse_kind(value: &str) -> Result<ArtifactKind> {
    match value {
        "markdown" | "guidance" => Ok(ArtifactKind::Markdown),
        "rust_binary" => Ok(ArtifactKind::RustBinary),
        "python_uv" => Ok(ArtifactKind::PythonUv),
        other => bail!("unknown skill kind `{other}`"),
    }
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

fn parse_output_format(value: &str) -> Result<OutputFormat> {
    match value {
        "text" => Ok(OutputFormat::Text),
        "json" => Ok(OutputFormat::Json),
        other => bail!("unknown output format `{other}`"),
    }
}

#[derive(Debug, Clone)]
struct Cursor {
    args: Vec<String>,
    index: usize,
}

impl Cursor {
    fn new(args: Vec<String>) -> Self {
        Self { args, index: 0 }
    }

    fn next(&mut self) -> Option<String> {
        let value = self.args.get(self.index).cloned()?;
        self.index += 1;
        Some(value)
    }

    fn next_required(&mut self, label: &str) -> Result<String> {
        self.next()
            .ok_or_else(|| anyhow::anyhow!("missing {label}"))
    }
}

fn print_usage() {
    println!(
        r#"Usage:
  skrun skill new --kind markdown|rust_binary|python_uv --id <id> [--name <name>] [--version <version>] <dir>
  skrun skill build [--profile release] [--target-dir <dir>] <dir>
  skrun skill run [--input <json-object>] [--timeout <seconds>] <dir>
  skrun skill run --id <id> [--root <dir>] [--input <json-object>] [--timeout <seconds>]
  skrun skill install-local [--root <dir>] [--id <id>] [--overwrite] <dir>
  skrun skill list [--root <dir>] [--format text|json]
  skrun skill show [--root <dir>] [--format text|json] <id>
  skrun catalog index <dir> [--out <file>]
  skrun catalog registry <dir> [--registry <file>] [--write]
  skrun catalog check <dir> [--registry <file>] [--index <file>]
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_skill_new_command() {
        let command = parse(vec![
            "skill".to_string(),
            "new".to_string(),
            "--kind".to_string(),
            "rust_binary".to_string(),
            "--id".to_string(),
            "regex-finder".to_string(),
            "skills/regex-finder".to_string(),
        ])
        .unwrap();

        let Command::Skill(SkillCommand::New(command)) = command else {
            panic!("expected skill new command");
        };
        assert_eq!(command.dir, PathBuf::from("skills/regex-finder"));
        assert_eq!(command.scaffold.id, "regex-finder");
        assert_eq!(command.scaffold.name, "Regex Finder");
        assert_eq!(command.scaffold.kind, ArtifactKind::RustBinary);
    }

    #[test]
    fn parses_skill_new_guidance_command() {
        let command = parse(vec![
            "skill".to_string(),
            "new".to_string(),
            "--kind".to_string(),
            "guidance".to_string(),
            "--id".to_string(),
            "team".to_string(),
            "skills/team".to_string(),
        ])
        .unwrap();

        let Command::Skill(SkillCommand::New(command)) = command else {
            panic!("expected skill new command");
        };
        assert_eq!(command.dir, PathBuf::from("skills/team"));
        assert_eq!(command.scaffold.id, "team");
        assert_eq!(command.scaffold.kind, ArtifactKind::Markdown);
    }

    #[test]
    fn parses_skill_run_input() {
        let command = parse(vec![
            "skill".to_string(),
            "run".to_string(),
            "--input".to_string(),
            r#"{"ok":true}"#.to_string(),
            "skills/echo".to_string(),
        ])
        .unwrap();

        let Command::Skill(SkillCommand::Run(command)) = command else {
            panic!("expected skill run command");
        };
        assert_eq!(command.input, serde_json::json!({ "ok": true }));
        assert_eq!(command.target, RunTarget::Dir(PathBuf::from("skills/echo")));
    }

    #[test]
    fn parses_skill_run_by_id() {
        let command = parse(vec![
            "skill".to_string(),
            "run".to_string(),
            "--id".to_string(),
            "echo".to_string(),
            "--root".to_string(),
            "installed".to_string(),
        ])
        .unwrap();

        let Command::Skill(SkillCommand::Run(command)) = command else {
            panic!("expected skill run command");
        };
        assert_eq!(
            command.target,
            RunTarget::Installed {
                root: PathBuf::from("installed"),
                id: "echo".to_string()
            }
        );
    }

    #[test]
    fn parses_skill_list_json_format() {
        let command = parse(vec![
            "skill".to_string(),
            "list".to_string(),
            "--root".to_string(),
            "installed".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ])
        .unwrap();

        let Command::Skill(SkillCommand::List(command)) = command else {
            panic!("expected skill list command");
        };
        assert_eq!(command.root, PathBuf::from("installed"));
        assert_eq!(command.format, OutputFormat::Json);
    }

    #[test]
    fn parses_catalog_index_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "index".to_string(),
            "skills".to_string(),
            "--out".to_string(),
            "skills.json".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Index(command)) = command else {
            panic!("expected catalog index command");
        };
        assert_eq!(command.dir, PathBuf::from("skills"));
        assert_eq!(command.out, Some(PathBuf::from("skills.json")));
    }

    #[test]
    fn parses_catalog_registry_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "registry".to_string(),
            "--registry".to_string(),
            "docs/REGISTRY.md".to_string(),
            "--write".to_string(),
            "skills".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Registry(command)) = command else {
            panic!("expected catalog registry command");
        };
        assert_eq!(command.dir, PathBuf::from("skills"));
        assert_eq!(command.registry, Some(PathBuf::from("docs/REGISTRY.md")));
        assert!(command.write);
    }

    #[test]
    fn parses_catalog_check_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "check".to_string(),
            "--index".to_string(),
            "skills.json".to_string(),
            "skills".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Check(command)) = command else {
            panic!("expected catalog check command");
        };
        assert_eq!(command.dir, PathBuf::from("skills"));
        assert_eq!(command.index, Some(PathBuf::from("skills.json")));
        assert_eq!(command.registry, None);
    }

    #[test]
    fn catalog_index_with_out_prints_nothing() {
        let dir = temp_dir("catalog-index-out");
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        let index = catalog::build_index(&dir).unwrap();
        let out = dir.join("skills.json");

        let printed = catalog_index_output(&index, Some(&out)).unwrap();

        assert!(printed.is_none());
        assert!(out.is_file());
    }

    #[test]
    fn catalog_registry_dry_run_fails_when_the_table_drifted() {
        let dir = drifted_registry_fixture("catalog-registry-dry-run");

        let code = run_catalog_registry(&CatalogRegistryCommand {
            dir: dir.clone(),
            registry: None,
            write: false,
        })
        .unwrap();

        assert_eq!(code, 1);
    }

    #[test]
    fn catalog_registry_write_succeeds_after_fixing_the_table() {
        let dir = drifted_registry_fixture("catalog-registry-write");

        let code = run_catalog_registry(&CatalogRegistryCommand {
            dir: dir.clone(),
            registry: None,
            write: true,
        })
        .unwrap();
        assert_eq!(code, 0);

        let code = run_catalog_registry(&CatalogRegistryCommand {
            dir: dir.clone(),
            registry: None,
            write: false,
        })
        .unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn catalog_registry_dry_run_fails_when_the_table_is_missing() {
        let dir = registry_without_table_fixture("catalog-registry-no-table-dry-run");

        let error = run_catalog_registry(&CatalogRegistryCommand {
            dir: dir.clone(),
            registry: None,
            write: false,
        })
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("has no `| Skill | When to read | Entrypoint |` table")
        );
    }

    #[test]
    fn catalog_registry_write_fails_when_the_table_is_missing() {
        let dir = registry_without_table_fixture("catalog-registry-no-table-write");
        let registry = dir.join(catalog::REGISTRY_FILE);
        let before = fs::read(&registry).unwrap();

        let error = run_catalog_registry(&CatalogRegistryCommand {
            dir: dir.clone(),
            registry: None,
            write: true,
        })
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("has no `| Skill | When to read | Entrypoint |` table")
        );
        assert_eq!(fs::read(&registry).unwrap(), before);
    }

    #[test]
    fn catalog_registry_fails_when_the_registry_cannot_be_read() {
        let dir = drifted_registry_fixture("catalog-registry-read-error");
        let registry = dir.join(catalog::REGISTRY_FILE);
        fs::remove_file(&registry).unwrap();
        fs::create_dir(&registry).unwrap();

        let error = run_catalog_registry(&CatalogRegistryCommand {
            dir,
            registry: None,
            write: true,
        })
        .unwrap_err();

        assert!(error.to_string().contains("read registry"));
    }

    fn registry_without_table_fixture(name: &str) -> PathBuf {
        let dir = temp_dir(name);
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        write(
            &dir.join(catalog::REGISTRY_FILE),
            concat!("# Skills\n", "\n", "The registry table is missing.\n",),
        );
        dir
    }

    fn drifted_registry_fixture(name: &str) -> PathBuf {
        let dir = temp_dir(name);
        write(
            &dir.join("demo/SKILL.md"),
            "---\nname: Demo\ndescription: A demo skill.\n---\n\n# Demo\n",
        );
        write(
            &dir.join(catalog::REGISTRY_FILE),
            concat!(
                "# Skills\n",
                "\n",
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n",
                "| Ghost | When ghosting | [ghost](ghost/SKILL.md) |\n",
                "\n",
                "Trailing text.\n"
            ),
        );
        dir
    }

    #[test]
    fn parses_skill_show_json_format() {
        let command = parse(vec![
            "skill".to_string(),
            "show".to_string(),
            "--root".to_string(),
            "installed".to_string(),
            "--format".to_string(),
            "json".to_string(),
            "team".to_string(),
        ])
        .unwrap();

        let Command::Skill(SkillCommand::Show(command)) = command else {
            panic!("expected skill show command");
        };
        assert_eq!(command.root, PathBuf::from("installed"));
        assert_eq!(command.id, "team");
        assert_eq!(command.format, OutputFormat::Json);
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
            std::env::temp_dir().join(format!("skrun-main-{name}-{}-{now}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
