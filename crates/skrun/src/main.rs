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
//! - catalog add and inspect commands
//! - catalog path and cat commands
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

use anyhow::{Context, Result};
use serde_json::Value;
use skrun::catalog;
use skrun::store;
use skrun::{
    ArtifactKind, BuildOptions, InstallOptions, RunOptions, ScaffoldOptions, build_skill,
    default_skills_dir, install_local_skill, list_installed_skills, load_artifact, run_skill,
    scaffold_skill,
};
use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("Error: {}", error_text(&error));
        std::process::exit(exit_code(&error));
    }
}

/// The text of an error and of the causes behind it.
///
/// Every cause of the error goes through the redaction helper and the text
/// of the whole chain goes through it as well, so a value of a broken
/// record that looks like a secret never reaches the output.
fn error_text(error: &anyhow::Error) -> String {
    let chain = error
        .chain()
        .map(|cause| catalog::redact(&cause.to_string()))
        .collect::<Vec<String>>()
        .join(": ");
    catalog::redact(&chain)
}

/// The exit status of a failed command.
///
/// A usage error, an error about how the command was invoked, ends the
/// command with status 2; every other error ends it with status 1.
fn exit_code(error: &anyhow::Error) -> i32 {
    if error
        .chain()
        .any(|cause| cause.is::<store::UnsafeVersion>())
    {
        return UNSAFE_EXIT_CODE;
    }
    let usage = error.chain().any(|cause| cause.is::<UsageError>());
    if usage { USAGE_EXIT_CODE } else { 1 }
}

/// The exit status of a usage error.
const USAGE_EXIT_CODE: i32 = 2;

/// The exit status of a version that is not safe to read.
///
/// A version that the user asked to read safely is refused with its own
/// status, so the user sees that the version is not safe and not a broken
/// file.
const UNSAFE_EXIT_CODE: i32 = 3;

/// An error about how the command was invoked.
///
/// A usage error is an error of the command line, not of the work itself, so
/// it ends the command with its own status.
#[derive(Debug, Clone)]
struct UsageError {
    message: String,
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for UsageError {}

/// Build a usage error.
fn usage_error(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(UsageError {
        message: message.into(),
    })
}

/// Build a usage error result.
fn usage<T>(message: impl Into<String>) -> Result<T> {
    Err(usage_error(message))
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
                catalog::redact_path(&options.dir)
            );
            Ok(())
        }
        Command::Skill(SkillCommand::Build(options)) => {
            let artifact = build_skill(&options.dir, &options.build)?;
            println!(
                "Built skill `{}` at {}",
                artifact.id,
                catalog::redact_path(&options.dir)
            );
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
                catalog::redact_path(&options.install.root)
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
            let code = run_catalog_check(&options)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Command::Catalog(CatalogCommand::Search(options)) => {
            let code = run_catalog_search(&options)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }

        Command::Catalog(CatalogCommand::Add(options)) => {
            run_catalog_add(&options)?;
            Ok(())
        }
        Command::Catalog(CatalogCommand::Inspect(options)) => {
            run_catalog_inspect(&options)?;
            Ok(())
        }
        Command::Catalog(CatalogCommand::Path(options)) => {
            run_catalog_path(&options)?;
            Ok(())
        }
        Command::Catalog(CatalogCommand::Cat(options)) => {
            run_catalog_cat(&options)?;
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
    Search(CatalogSearchCommand),
    Add(CatalogAddCommand),
    Inspect(CatalogInspectCommand),
    Path(CatalogPathCommand),
    Cat(CatalogCatCommand),
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

#[derive(Debug, Clone, PartialEq)]
struct CatalogSearchCommand {
    dir: PathBuf,
    query: String,
    kind: Option<catalog::SkillKind>,
    has_scripts: bool,
    limit: usize,
    json: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct CatalogAddCommand {
    source: PathBuf,
    store: Option<PathBuf>,
    id: Option<String>,
    from: Option<String>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct CatalogInspectCommand {
    id: Option<String>,
    store: Option<PathBuf>,
    list: bool,
    json: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct CatalogPathCommand {
    id: String,
    store: Option<PathBuf>,
    version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct CatalogCatCommand {
    id: String,
    file: Option<String>,
    store: Option<PathBuf>,
    version: Option<String>,
    list: bool,
    require_safe: bool,
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
        other => usage(format!("unknown command `{}`", catalog::redact(other))),
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
        other => usage(format!(
            "unknown skill command `{}`",
            catalog::redact(other)
        )),
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
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => dir = Some(PathBuf::from(value)),
        }
    }

    let kind = kind.ok_or_else(|| usage_error("missing --kind"))?;
    let id = id.ok_or_else(|| usage_error("missing --id"))?;
    let name = name.unwrap_or_else(|| title_from_id(&id));
    let dir = dir.ok_or_else(|| usage_error("missing skill directory"))?;
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
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => dir = Some(PathBuf::from(value)),
        }
    }

    Ok(BuildCommand {
        dir: dir.ok_or_else(|| usage_error("missing skill directory"))?,
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
                    return usage("--input must be a JSON object");
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
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => dir = Some(PathBuf::from(value)),
        }
    }

    let target = match (dir, id) {
        (Some(_), Some(_)) => {
            return usage("skill run accepts either --id or a directory, not both");
        }
        (Some(dir), None) => RunTarget::Dir(dir),
        (None, Some(id)) => RunTarget::Installed {
            root: root.unwrap_or(default_skills_dir()?),
            id,
        },
        (None, None) => return usage("missing skill directory or --id"),
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
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => source = Some(PathBuf::from(value)),
        }
    }

    let root = root.unwrap_or(default_skills_dir()?);
    let mut install = InstallOptions::new(root).with_overwrite(overwrite);
    if let Some(skill_id) = skill_id {
        install = install.with_skill_id(skill_id);
    }

    Ok(InstallLocalCommand {
        source: source.ok_or_else(|| usage_error("missing source skill directory"))?,
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
            value => {
                return usage(format!("unknown argument `{}`", catalog::redact(value)));
            }
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
            value if value.starts_with('-') => {
                return usage(format!("unknown argument `{}`", catalog::redact(value)));
            }
            value => id = Some(value.to_string()),
        }
    }
    Ok(ShowCommand {
        root: root.unwrap_or(default_skills_dir()?),
        id: id.ok_or_else(|| usage_error("missing skill id"))?,
        format,
    })
}

fn parse_catalog(mut cursor: Cursor) -> Result<CatalogCommand> {
    match cursor.next_required("catalog command")?.as_str() {
        "index" => parse_catalog_index(cursor).map(CatalogCommand::Index),
        "registry" => parse_catalog_registry(cursor).map(CatalogCommand::Registry),
        "check" => parse_catalog_check(cursor).map(CatalogCommand::Check),
        "search" => parse_catalog_search(cursor).map(CatalogCommand::Search),
        "add" => parse_catalog_add(cursor).map(CatalogCommand::Add),
        "inspect" => parse_catalog_inspect(cursor).map(CatalogCommand::Inspect),
        "path" => parse_catalog_path(cursor).map(CatalogCommand::Path),
        "cat" => parse_catalog_cat(cursor).map(CatalogCommand::Cat),
        other => usage(format!(
            "unknown catalog command `{}`",
            catalog::redact(other)
        )),
    }
}

fn parse_catalog_index(mut cursor: Cursor) -> Result<CatalogIndexCommand> {
    let mut out = None;
    let mut dir = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(cursor.next_required("--out value")?)),
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => dir = Some(PathBuf::from(value)),
        }
    }

    Ok(CatalogIndexCommand {
        dir: dir.ok_or_else(|| usage_error("missing skills directory"))?,
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
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => dir = Some(PathBuf::from(value)),
        }
    }

    Ok(CatalogRegistryCommand {
        dir: dir.ok_or_else(|| usage_error("missing skills directory"))?,
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
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => dir = Some(PathBuf::from(value)),
        }
    }

    Ok(CatalogCheckCommand {
        dir: dir.ok_or_else(|| usage_error("missing skills directory"))?,
        registry,
        index,
    })
}

/// Parse the `catalog search` command.
///
/// The first positional argument is the skills directory, every other
/// positional argument is a part of the query. The query is matched as one
/// text, so a Chinese term and an English term can be searched for together.
fn parse_catalog_search(mut cursor: Cursor) -> Result<CatalogSearchCommand> {
    let mut kind = None;
    let mut has_scripts = false;
    let mut limit = catalog::SEARCH_LIMIT;
    let mut json = false;
    let mut dir = None;
    let mut query = Vec::new();

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--kind" => kind = Some(parse_search_kind(&cursor.next_required("--kind value")?)?),
            "--has-scripts" => has_scripts = true,
            "--limit" => limit = parse_limit(&cursor.next_required("--limit value")?)?,
            "--json" => json = true,
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => {
                if dir.is_none() {
                    dir = Some(PathBuf::from(value));
                } else {
                    query.push(value.to_string());
                }
            }
        }
    }

    let dir = dir.ok_or_else(|| usage_error("missing skills directory"))?;
    let query = query.join(" ");
    if query.split_whitespace().next().is_none() {
        return usage("missing query");
    }

    Ok(CatalogSearchCommand {
        dir,
        query,
        kind,
        has_scripts,
        limit,
        json,
    })
}

/// Parse the `catalog add` command.
///
/// The source is a local skill directory or a zip archive. The store, the id
/// and the provenance of the skill are optional.
fn parse_catalog_add(mut cursor: Cursor) -> Result<CatalogAddCommand> {
    let mut store = None;
    let mut id = None;
    let mut from = None;
    let mut json = false;
    let mut source = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--store" => store = Some(PathBuf::from(cursor.next_required("--store value")?)),
            "--id" => id = Some(cursor.next_required("--id value")?),
            "--from" => from = Some(cursor.next_required("--from value")?),
            "--json" => json = true,
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => {
                if source.is_none() {
                    source = Some(PathBuf::from(value));
                } else {
                    return usage("catalog add takes one source");
                }
            }
        }
    }

    Ok(CatalogAddCommand {
        source: source.ok_or_else(|| usage_error("missing source"))?,
        store,
        id,
        from,
        json,
    })
}

/// Parse the `catalog inspect` command.
///
/// The command shows the record of one skill id, or the list of every stored
/// id when `--list` is given.
fn parse_catalog_inspect(mut cursor: Cursor) -> Result<CatalogInspectCommand> {
    let mut store = None;
    let mut id = None;
    let mut list = false;
    let mut json = false;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--store" => store = Some(PathBuf::from(cursor.next_required("--store value")?)),
            "--list" => list = true,
            "--json" => json = true,
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => {
                if id.is_none() {
                    id = Some(value.to_string());
                } else {
                    return usage("catalog inspect takes one skill id");
                }
            }
        }
    }

    if !list && id.is_none() {
        return usage("missing skill id");
    }

    Ok(CatalogInspectCommand {
        id,
        store,
        list,
        json,
    })
}

/// Parse the `catalog path` command.
///
/// The command shows the folder of a stored version of a skill. The newest
/// version is shown when no version is named.
fn parse_catalog_path(mut cursor: Cursor) -> Result<CatalogPathCommand> {
    let mut store = None;
    let mut version = None;
    let mut id = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--store" => store = Some(PathBuf::from(cursor.next_required("--store value")?)),
            "--version" => version = Some(cursor.next_required("--version value")?),
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => {
                if id.is_none() {
                    id = Some(value.to_string());
                } else {
                    return usage("catalog path takes one skill id");
                }
            }
        }
    }

    Ok(CatalogPathCommand {
        id: id.ok_or_else(|| usage_error("missing skill id"))?,
        store,
        version,
    })
}

/// Parse the `catalog cat` command.
///
/// The command shows a file of a stored version of a skill. The file of the
/// skill itself is shown when no file is named, and the list of the files of
/// markdown is shown when `--list` is given.
fn parse_catalog_cat(mut cursor: Cursor) -> Result<CatalogCatCommand> {
    let mut store = None;
    let mut version = None;
    let mut list = false;
    let mut require_safe = false;
    let mut id = None;
    let mut file = None;

    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--store" => store = Some(PathBuf::from(cursor.next_required("--store value")?)),
            "--version" => version = Some(cursor.next_required("--version value")?),
            "--list" => list = true,
            "--require-safe" => require_safe = true,
            value if value.starts_with('-') => {
                return usage(format!("unknown option `{}`", catalog::redact(value)));
            }
            value => {
                if id.is_none() {
                    id = Some(value.to_string());
                } else if file.is_none() {
                    file = Some(value.to_string());
                } else {
                    return usage("catalog cat takes one skill id and one file");
                }
            }
        }
    }

    Ok(CatalogCatCommand {
        id: id.ok_or_else(|| usage_error("missing skill id"))?,
        file,
        store,
        version,
        list,
        require_safe,
    })
}

/// The kind of skill a search keeps.
fn parse_search_kind(value: &str) -> Result<catalog::SkillKind> {
    match value {
        "folder" => Ok(catalog::SkillKind::Folder),
        "flat" => Ok(catalog::SkillKind::Flat),
        other => usage(format!("unknown search kind `{}`", catalog::redact(other))),
    }
}

/// The number of results a search keeps.
fn parse_limit(value: &str) -> Result<usize> {
    let limit = value.parse::<usize>().map_err(|_| {
        usage_error(format!(
            "invalid --limit value `{}`",
            catalog::redact(value)
        ))
    })?;
    if limit < 1 {
        return usage("--limit must be at least 1");
    }
    Ok(limit)
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
    let mut stdout = std::io::stdout().lock();
    run_catalog_registry_to(options, &mut stdout)
}

/// Run the `catalog registry` command and write its report to `out`.
///
/// The report is the only thing the command prints; the status follows the
/// mode.
fn run_catalog_registry_to(options: &CatalogRegistryCommand, out: &mut dyn Write) -> Result<i32> {
    let registry = options
        .registry
        .clone()
        .unwrap_or_else(|| options.dir.join(catalog::REGISTRY_FILE));
    let report = catalog::reconcile_registry(&options.dir, &registry, options.write)?;
    write!(out, "{}", report.render())?;
    out.flush()?;
    Ok(report.exit_code(options.write))
}

/// Run the `catalog search` command and return its exit status.
///
/// The results are the only thing the command prints; the status is 1 when
/// no skill matches the query.
fn run_catalog_search(options: &CatalogSearchCommand) -> Result<i32> {
    let mut stdout = std::io::stdout().lock();
    run_catalog_search_to(options, &mut stdout)
}

/// Run the `catalog search` command and write its results to `out`.
///
/// The results are the only thing the command prints; the status is 1 when
/// no skill matches the query.
fn run_catalog_search_to(options: &CatalogSearchCommand, out: &mut dyn Write) -> Result<i32> {
    let search = catalog::SearchOptions {
        query: options.query.clone(),
        kind: options.kind,
        has_scripts: options.has_scripts,
        limit: options.limit,
    };
    let hits = catalog::search_skills(&options.dir, &search)?;
    if options.json {
        write!(out, "{}", catalog::render_search_json(&hits)?)?;
    } else {
        write!(out, "{}", catalog::render_search_text(&hits))?;
    }
    out.flush()?;
    Ok(if hits.is_empty() { 1 } else { 0 })
}

/// Run the `catalog check` command and return its exit status.
///
/// The report is the only thing the command prints; the status is 1 when the
/// check found an error.
fn run_catalog_check(options: &CatalogCheckCommand) -> Result<i32> {
    let mut stdout = std::io::stdout().lock();
    run_catalog_check_to(options, &mut stdout)
}

/// Run the `catalog check` command and write its report to `out`.
fn run_catalog_check_to(options: &CatalogCheckCommand, out: &mut dyn Write) -> Result<i32> {
    let report = catalog::check_catalog(
        &options.dir,
        options.registry.as_deref(),
        options.index.as_deref(),
    )?;
    write!(out, "{}", report.render())?;
    out.flush()?;
    Ok(if report.has_errors() { 1 } else { 0 })
}

/// Run the `catalog add` command.
///
/// The skill is copied into the store, hashed and scanned. Nothing of the
/// skill is ever run, so an added skill is safe to look at.
fn run_catalog_add(options: &CatalogAddCommand) -> Result<()> {
    let result = store::add_skill(&store::AddOptions {
        source: options.source.clone(),
        store: options.store.clone(),
        id: options.id.clone(),
        from: options.from.clone(),
    })?;
    if options.json {
        let value = serde_json::to_value(&result.version)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&store::redact_json_strings(&value))?
        );
    } else {
        let state = if result.already_stored {
            if result.refreshed {
                "already stored (record refreshed)"
            } else {
                "already stored"
            }
        } else {
            "stored"
        };
        println!(
            "{} {} {} ({} files, {} findings) tier {}",
            state,
            catalog::redact(&result.id),
            catalog::redact(&result.version.hash),
            result.version.files,
            result.version.findings.len(),
            catalog::redact(result.version.tier.as_str())
        );
    }
    Ok(())
}

/// Run the `catalog inspect` command.
///
/// The record of the skill is the only thing the command prints. A skill that
/// was never added has no record, so the command fails.
fn run_catalog_inspect(options: &CatalogInspectCommand) -> Result<()> {
    let root = store::resolve_store_root(options.store.as_deref())?;
    if options.list {
        let summaries = store::list_records(&root)?;
        if options.json {
            println!("{}", serde_json::to_string_pretty(&summaries)?);
        } else {
            print!("{}", store::render_summaries(&summaries));
        }
        return Ok(());
    }

    let id = options.id.clone().unwrap_or_default();
    let record = store::read_record(&root, &id)?;
    if options.json {
        print!("{}", store::redacted_record_json(&record)?);
    } else {
        print!("{}", store::render_record(&record));
    }
    Ok(())
}

/// Run the `catalog path` command.
///
/// The folder of the stored version is the only thing the command prints, so
/// the path can be used by another tool without reading the record again.
fn run_catalog_path(options: &CatalogPathCommand) -> Result<()> {
    let root = store::resolve_store_root(options.store.as_deref())?;
    let location = store::version_location(&root, &options.id, options.version.as_deref())?;
    // The folder of the version has to be there, so the path that is
    // printed is a folder of the store and not a folder that is gone.
    let folder = store::stored_folder(&location)?;
    println!("{}", catalog::redact_path(&folder));
    Ok(())
}

/// Run the `catalog cat` command.
///
/// The file of the stored version is the only thing the command prints. The
/// file is read and never run, so looking at a skill is safe.
///
/// With `--require-safe` a version that is not safe to read is refused
/// before anything is printed, so nothing of such a version reaches the
/// output.
fn run_catalog_cat(options: &CatalogCatCommand) -> Result<()> {
    let root = store::resolve_store_root(options.store.as_deref())?;
    // The version is resolved once, so the check of the safety and the
    // read of the file work on the same version, even when another
    // version is added while the command runs.
    let location = store::version_location(&root, &options.id, options.version.as_deref())?;
    if options.require_safe {
        store::refuse_unsafe_version(&location.version)?;
    }
    if options.list {
        let files = store::list_version_markdown(&location)?;
        let mut text = String::new();
        for file in files {
            text.push_str(&catalog::redact(&file));
            text.push('\n');
        }
        print!("{text}");
        return Ok(());
    }
    let file = match options.file.as_deref() {
        Some(file) if !file.trim().is_empty() => file.to_string(),
        _ => store::SKILL_FILE.to_string(),
    };
    let text = store::read_version_file(&location, &file)?;
    print!("{text}");
    Ok(())
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
        other => usage(format!("unknown skill kind `{}`", catalog::redact(other))),
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
        other => usage(format!(
            "unknown output format `{}`",
            catalog::redact(other)
        )),
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
            .ok_or_else(|| usage_error(format!("missing {label}")))
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
  skrun catalog search <dir> <query>... [--kind folder|flat] [--has-scripts] [--limit N] [--json]
  skrun catalog add <source> [--store <dir>] [--id <id>] [--from <provenance>] [--json]
  skrun catalog inspect <id> [--store <dir>] [--json]
  skrun catalog inspect --list [--store <dir>] [--json]
  skrun catalog path <id> [--store <dir>] [--version <hash>]
  skrun catalog cat <id> [<relative-file>] [--store <dir>] [--version <hash>] [--require-safe]
  skrun catalog cat <id> --list [--store <dir>] [--version <hash>]
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A key shaped value that must never reach the command output.
    const SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

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
    fn parses_catalog_add_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "add".to_string(),
            "skills/demo".to_string(),
            "--store".to_string(),
            "store".to_string(),
            "--id".to_string(),
            "demo".to_string(),
            "--from".to_string(),
            "https://example.com/repo".to_string(),
            "--json".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Add(command)) = command else {
            panic!("expected catalog add command");
        };
        assert_eq!(command.source, PathBuf::from("skills/demo"));
        assert_eq!(command.store, Some(PathBuf::from("store")));
        assert_eq!(command.id, Some("demo".to_string()));
        assert_eq!(command.from, Some("https://example.com/repo".to_string()));
        assert!(command.json);
    }

    #[test]
    fn catalog_add_needs_a_source() {
        let error = parse(vec!["catalog".to_string(), "add".to_string()]).unwrap_err();

        let printed = format!("{error:#}");
        assert!(printed.contains("missing source"), "{printed}");
    }

    #[test]
    fn parses_catalog_inspect_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "inspect".to_string(),
            "demo".to_string(),
            "--store".to_string(),
            "store".to_string(),
            "--json".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Inspect(command)) = command else {
            panic!("expected catalog inspect command");
        };
        assert_eq!(command.id, Some("demo".to_string()));
        assert_eq!(command.store, Some(PathBuf::from("store")));
        assert!(!command.list);
        assert!(command.json);
    }

    #[test]
    fn parses_catalog_inspect_list_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "inspect".to_string(),
            "--list".to_string(),
            "--store".to_string(),
            "store".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Inspect(command)) = command else {
            panic!("expected catalog inspect command");
        };
        assert!(command.list);
        assert_eq!(command.id, None);
        assert_eq!(command.store, Some(PathBuf::from("store")));
    }

    #[test]
    fn catalog_commands_redact_an_unknown_option() {
        for command in ["index", "registry", "check"] {
            let error = parse(vec![
                "catalog".to_string(),
                command.to_string(),
                "skills".to_string(),
                format!("--{SECRET}"),
            ])
            .unwrap_err();

            let printed = format!("{error:#}");
            assert!(!printed.contains(SECRET), "{printed}");
            assert!(printed.contains("unknown option `<redacted>`"), "{printed}");
        }
    }

    #[test]
    fn catalog_commands_redact_an_unknown_command() {
        let error = parse(vec![
            "catalog".to_string(),
            SECRET.to_string(),
            "skills".to_string(),
        ])
        .unwrap_err();

        let printed = format!("{error:#}");
        assert!(!printed.contains(SECRET), "{printed}");
        assert!(
            printed.contains("unknown catalog command `<redacted>`"),
            "{printed}"
        );
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

    #[test]
    fn catalog_check_never_prints_a_secret() {
        let dir = secret_fixture("catalog-check-secret");
        let mut out = Vec::new();

        let code = run_catalog_check_to(
            &CatalogCheckCommand {
                dir: dir.clone(),
                registry: None,
                index: None,
            },
            &mut out,
        )
        .unwrap();

        let printed = String::from_utf8(out).unwrap();
        assert_eq!(code, 1);
        assert!(!printed.contains(SECRET), "{printed}");
        assert!(printed.contains("<redacted>"), "{printed}");
        // The finding keeps the location of the secret, never its value.
        assert!(
            printed.contains("possible secret in `demo/SKILL.md:2`"),
            "{printed}"
        );
        assert!(
            printed.contains("has frontmatter name `<redacted>`"),
            "{printed}"
        );
        assert!(
            printed.contains("has a broken link `<redacted>`"),
            "{printed}"
        );
    }

    #[test]
    fn catalog_registry_never_prints_a_secret() {
        let dir = secret_fixture("catalog-registry-secret");
        let mut out = Vec::new();

        let code = run_catalog_registry_to(
            &CatalogRegistryCommand {
                dir: dir.clone(),
                registry: None,
                write: false,
            },
            &mut out,
        )
        .unwrap();

        let printed = String::from_utf8(out).unwrap();
        assert_eq!(code, 1);
        assert!(!printed.contains(SECRET), "{printed}");
        assert!(printed.contains("<redacted>"), "{printed}");
    }

    #[test]
    fn catalog_registry_error_never_prints_a_secret() {
        let dir = secret_fixture("catalog-registry-error");
        let registry = dir.join(SECRET);
        fs::write(&registry, "# Skills\n").unwrap();

        let error = run_catalog_registry_to(
            &CatalogRegistryCommand {
                dir: dir.clone(),
                registry: Some(registry.clone()),
                write: false,
            },
            &mut Vec::new(),
        )
        .unwrap_err();

        let printed = format!("{error:#}");
        assert!(!printed.contains(SECRET), "{printed}");
        assert!(printed.contains("<redacted>"), "{printed}");
    }

    #[test]
    fn catalog_check_and_dry_run_do_not_write_files() {
        let dir = drifted_registry_fixture("catalog-no-write");
        let before = snapshot(&dir);
        let mut out = Vec::new();

        run_catalog_check_to(
            &CatalogCheckCommand {
                dir: dir.clone(),
                registry: None,
                index: None,
            },
            &mut out,
        )
        .unwrap();
        run_catalog_registry_to(
            &CatalogRegistryCommand {
                dir: dir.clone(),
                registry: None,
                write: false,
            },
            &mut out,
        )
        .unwrap();

        assert_eq!(snapshot(&dir), before);
    }

    #[test]
    fn catalog_registry_round_trips_a_pipe_in_the_skill_name() {
        let dir = temp_dir("catalog-registry-pipe");
        write(
            &dir.join("pipe/SKILL.md"),
            "---\nname: Pipe | Skill\ndescription: A | B summary.\n---\n\n# Pipe\n",
        );
        write(
            &dir.join(catalog::REGISTRY_FILE),
            "# Skills\n\n| Skill | When to read | Entrypoint |\n| --- | --- | --- |\n",
        );
        let mut out = Vec::new();

        let code = run_catalog_registry_to(
            &CatalogRegistryCommand {
                dir: dir.clone(),
                registry: None,
                write: true,
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(code, 0);

        let registry = fs::read_to_string(dir.join(catalog::REGISTRY_FILE)).unwrap();
        assert!(
            registry.contains("| Pipe \\| Skill | A \\| B summary. | [pipe](pipe/SKILL.md) |"),
            "{registry}"
        );

        let code = run_catalog_registry_to(
            &CatalogRegistryCommand {
                dir: dir.clone(),
                registry: None,
                write: false,
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(code, 0);
    }

    /// A skills directory whose frontmatter name, link target and registry row
    /// all hold a key shaped value.
    fn secret_fixture(name: &str) -> PathBuf {
        let dir = temp_dir(name);
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
        write(
            &dir.join(catalog::REGISTRY_FILE),
            concat!(
                "# Skills\n",
                "\n",
                "| Skill | When to read | Entrypoint |\n",
                "| --- | --- | --- |\n",
                "| Broken | When breaking | AKIAIOSFODNN7EXAMPLE |\n",
                "| Also broken | AKIAIOSFODNN7EXAMPLE |\n",
            ),
        );
        dir
    }

    /// Every file of a directory with its content, sorted by path.
    fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files = Vec::new();
        collect_files(root, root, &mut files);
        files.sort();
        files
    }

    fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(root, &path, out);
            } else {
                let name = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                out.push((name, fs::read(&path).unwrap_or_default()));
            }
        }
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
    fn parses_catalog_search_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "search".to_string(),
            "--kind".to_string(),
            "folder".to_string(),
            "--has-scripts".to_string(),
            "--limit".to_string(),
            "5".to_string(),
            "--json".to_string(),
            "skills".to_string(),
            "ai".to_string(),
            "\u{6a21}\u{578b}".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Search(command)) = command else {
            panic!("expected catalog search command");
        };
        assert_eq!(command.dir, PathBuf::from("skills"));
        assert_eq!(command.query, "ai \u{6a21}\u{578b}");
        assert_eq!(command.kind, Some(catalog::SkillKind::Folder));
        assert!(command.has_scripts);
        assert_eq!(command.limit, 5);
        assert!(command.json);
    }

    #[test]
    fn a_search_without_a_query_is_a_usage_error() {
        let error = parse(vec![
            "catalog".to_string(),
            "search".to_string(),
            "skills".to_string(),
        ])
        .unwrap_err();

        assert!(error.is::<UsageError>(), "{error:?}");
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

    #[test]
    fn parses_catalog_path_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "path".to_string(),
            "demo".to_string(),
            "--store".to_string(),
            "store".to_string(),
            "--version".to_string(),
            "0123456789abcdef".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Path(command)) = command else {
            panic!("expected catalog path command");
        };
        assert_eq!(command.id, "demo");
        assert_eq!(command.store, Some(PathBuf::from("store")));
        assert_eq!(command.version, Some("0123456789abcdef".to_string()));
    }

    #[test]
    fn parses_catalog_cat_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "cat".to_string(),
            "demo".to_string(),
            "docs/guide.md".to_string(),
            "--store".to_string(),
            "store".to_string(),
            "--version".to_string(),
            "0123456789abcdef".to_string(),
            "--require-safe".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Cat(command)) = command else {
            panic!("expected catalog cat command");
        };
        assert_eq!(command.id, "demo");
        assert_eq!(command.file, Some("docs/guide.md".to_string()));
        assert_eq!(command.store, Some(PathBuf::from("store")));
        assert_eq!(command.version, Some("0123456789abcdef".to_string()));
        assert!(command.require_safe);
        assert!(!command.list);
    }

    #[test]
    fn parses_catalog_cat_list_command() {
        let command = parse(vec![
            "catalog".to_string(),
            "cat".to_string(),
            "demo".to_string(),
            "--list".to_string(),
        ])
        .unwrap();

        let Command::Catalog(CatalogCommand::Cat(command)) = command else {
            panic!("expected catalog cat command");
        };
        assert_eq!(command.id, "demo");
        assert!(command.list);
        assert_eq!(command.file, None);
    }

    #[test]
    fn a_catalog_path_without_an_id_is_a_usage_error() {
        let error = parse(vec!["catalog".to_string(), "path".to_string()]).unwrap_err();

        assert!(error.is::<UsageError>(), "{error:?}");
    }

    #[test]
    fn a_catalog_cat_without_an_id_is_a_usage_error() {
        let error = parse(vec!["catalog".to_string(), "cat".to_string()]).unwrap_err();

        assert!(error.is::<UsageError>(), "{error:?}");
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
