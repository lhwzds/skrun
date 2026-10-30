---
title: How It Works
---

# How It Works

<sub>**English** · [中文](zh/04-how-it-works.md)</sub>

Every operation starts the same way: `load_artifact` reads the skill directory into a validated `SkillArtifact`. `build_skill` and `run_skill` then dispatch on its `kind`; `install_local_skill` copies the directory. There is no daemon, cache or registry; the filesystem is the only state.

<sub>Sources: [runtime.rs:594-609](../crates/skrun/src/runtime.rs#L594-L609)</sub>

## Loading a skill directory

If `artifact.json` exists, it is decoded and then `apply_skill_markdown` fills only the metadata fields the manifest left empty (`description`, `tags`, `suggested_tools`, `content`, `source_ref`) from `SKILL.md`. The manifest always wins; `SKILL.md` is the human-facing description that agents can read. Without `artifact.json`, the directory must contain `SKILL.md` and becomes a `markdown` skill named after the directory. Either way `validate` runs last, so every caller — CLI, Rust library, Python — sees the same errors for a bad manifest.

<sub>Sources: [runtime.rs:594-609](../crates/skrun/src/runtime.rs#L594-L609) · [runtime.rs:427-447](../crates/skrun/src/runtime.rs#L427-L447) · [runtime.rs:392-425](../crates/skrun/src/runtime.rs#L392-L425)</sub>

## How a call runs

```mermaid
sequenceDiagram
  participant C as caller
  participant R as run_skill
  participant P as skill process
  C->>R: dir, JSON value, RunOptions
  R->>R: canonicalize dir, load_artifact
  R->>P: spawn in dir (rust: entry binary / python: uv run --project dir python entry)
  R->>P: stdin = JSON + "\n", then close
  loop every 10 ms until exit or timeout
    R->>P: try_wait
  end
  P-->>R: exit status, stdout, stderr
  R->>R: non-zero exit -> error; stdout must parse as a JSON object
  R-->>C: SkillRunOutput { value, stderr, exit_code }
```

The working directory is the skill directory, so a skill can open its own files by relative path. Closing stdin after one value is what lets a skill simply read to EOF. Stderr is captured and returned as diagnostics, never parsed.

<sub>Sources: [runtime.rs:675-717](../crates/skrun/src/runtime.rs#L675-L717) · [runtime.rs:984-1044](../crates/skrun/src/runtime.rs#L984-L1044)</sub>

## What counts as failure

- **Timeout**: after `timeout` (60 s by default) the process is killed and the call fails with `skill process timed out after Ns`.
- **Non-zero exit**: the call fails with the exit code and trimmed stderr. Stdout is discarded, so a skill that prints a structured error body and exits 1 (as `regex-finder` does) loses that body; put the message on stderr too, or exit 0 with `"ok": false`.
- **Bad output**: stdout must be exactly one JSON value and it must be an object; arrays, scalars or several lines of JSON are errors.

Stdout and stderr are only collected after the process exits. Untested, from reading the code: a skill that writes more than the OS pipe buffer before exiting would block until the timeout. To be safe, keep outputs small or write large results to a file and return its path.

<sub>Sources: [runtime.rs:1005-1044](../crates/skrun/src/runtime.rs#L1005-L1044) · [main.rs:27-35](../examples/skills/regex-finder/src/main.rs#L27-L35)</sub>

## Builds

`rust_binary`: run `cargo build --manifest-path <dir>/Cargo.toml --target-dir <target>` with `--release` when the profile is `release`, otherwise `--profile <name>`, then copy `<target>/<profile>/<id>` (plus the platform exe suffix) to `entry`. The binary name must therefore equal the skill id. The copy goes to `entry` whatever the profile, so a debug build still lands in `bin/release/...` with the scaffolded entry.

`python_uv`: run `uv lock --project <dir>` only when `uv.lock` is missing, then `uv sync --project <dir> --locked`. The lock file is part of the skill, so installs are reproducible.

Both build steps capture output and report only stderr on failure. `markdown` skills cannot be built.

<sub>Sources: [runtime.rs:661-673](../crates/skrun/src/runtime.rs#L661-L673) · [runtime.rs:932-982](../crates/skrun/src/runtime.rs#L932-L982) · [runtime.rs:1046-1058](../crates/skrun/src/runtime.rs#L1046-L1058)</sub>

## Install and list

`install_local_skill` validates the source, then recursively copies it to `<root>/<id>`. An existing target fails with `installed skill already exists` unless `overwrite` is set; with `overwrite`, the target is deleted first, and installing a directory onto itself returns early without copying. The copy is re-loaded and returned, so a broken copy fails immediately. Everything in the source directory is copied, including built binaries and any `.venv`. Renaming with `--id` changes only the directory name; the copied `artifact.json` keeps its original `id`.

`list_installed_skills` scans the root's direct subdirectories that hold `artifact.json` or `SKILL.md`, silently skips those that fail to load, and sorts by id. A missing root lists as empty. Use `skill show <id>` to see the load error for a skill that is missing from the list.

<sub>Sources: [runtime.rs:727-773](../crates/skrun/src/runtime.rs#L727-L773) · [runtime.rs:1064-1085](../crates/skrun/src/runtime.rs#L1064-L1085) · [runtime.rs:760-766](../crates/skrun/src/runtime.rs#L760-L766)</sub>

## The Python boundary

`skrun-native` exposes five functions (`runtime_load_artifact_json`, `runtime_list_skills_json`, `runtime_build_skill_json`, `runtime_run_skill_json`, `runtime_install_local_skill_json`). Each takes strings and returns a JSON string, so the ABI is just serde JSON and the Python dataclasses mirror the Rust structs. Each call runs inside `py.detach`, which releases the GIL while a build or skill process runs, so other Python threads keep working. Rust errors become `RuntimeError` and the SDK rewraps them as `SkillRuntimeError`. An empty or missing `root` falls back to `default_skills_dir`. Builds from Python always use the default Cargo/uv commands and release profile; only `target_dir` is configurable.

<sub>Sources: [lib.rs:39-151](../crates/py/src/lib.rs#L39-L151) · [skill.py:316-331](../python/skrun/skill.py#L316-L331)</sub>
