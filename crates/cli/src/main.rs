//! # codocia
//!
//! CLI owns the minimal executable skill command loop.
//!
//! ## Owns
//! - skill artifact scaffolding command
//! - skill artifact build command
//! - skill artifact run command
//! - local skill install and list commands
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
//! - runtime
//!
//! ## Verify
//! - cargo test -p cli

use anyhow::{Context, Result, bail};
use runtime::{
    ArtifactKind, BuildOptions, InstallOptions, RunOptions, ScaffoldOptions, build_skill,
    default_skills_dir, install_local_skill, list_installed_skills, load_artifact, run_skill,
    scaffold_skill,
};
use serde_json::Value;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
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
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Help,
    Skill(SkillCommand),
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
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
