---
title: Reference
---

# Reference

<sub>**English** · [中文](zh/03-reference.md)</sub>

Exact names for the CLI, environment, file formats and Python API, copied from the code. `skrun`, `skrun -h`, `skrun --help` and `skrun help` print the usage text.

<sub>Sources: [main.rs:248-251](../crates/skrun/src/main.rs#L248-L251) · [main.rs:639-654](../crates/skrun/src/main.rs#L639-L654)</sub>

## CLI commands and flags

Skill commands live under `skrun skill`; `skrun catalog` is below. Positional `<dir>` / `<id>` may appear anywhere among the flags.

| command | flags (default) |
|---|---|
| `new <dir>` | `--kind markdown\|guidance\|rust_binary\|python_uv` (required), `--id` (required), `--name` (title-cased id: `regex-finder` → `Regex Finder`), `--version` (`0.1.0`) |
| `build <dir>` | `--profile` (`release`), `--target-dir`, `--cargo` (`cargo`), `--uv` (`uv`) |
| `run <dir>` or `run --id <id>` | `--input` (`{}`; must be a JSON object), `--timeout` seconds (`60`), `--uv` (`uv`), `--root` (with `--id` only) |
| `install-local <dir>` | `--root`, `--id` (rename on install), `--overwrite` |
| `list` | `--root`, `--format text\|json` (`text`) |
| `show <id>` | `--root`, `--format text\|json` (`text`) |

`--root` defaults to the skills root (see below). Text output of `list` and `show` is tab-separated `id kind version name`. Passing both a directory and `--id` to `run` is an error. `--cargo` and `--uv` exist but are not in the usage text. Errors print `Error: ...` to stderr and exit 1; echoed arguments and paths that look like a secret print as `<redacted>`.

<sub>Sources: [main.rs:47-52](../crates/skrun/src/main.rs#L47-L52) · [main.rs:272-301](../crates/skrun/src/main.rs#L272-L301) · [main.rs:303-368](../crates/skrun/src/main.rs#L303-L368) · [main.rs:370-433](../crates/skrun/src/main.rs#L370-L433) · [main.rs:577-598](../crates/skrun/src/main.rs#L577-L598) · +1</sub>

## Catalog commands

`skrun catalog` works on a directory of guidance skills: non-hidden subdirectories with a `SKILL.md`, plus top-level `*.md` except `SKILL_REGISTRY.md`, `README.md`, `README_CN.md`, `AGENTS.md` and `CLAUDE.md`.

| command | flags (default) | exits 1 when |
|---|---|---|
| `index <dir>` | `--out <file>` (stdout) | the directory cannot be read |
| `registry <dir>` | `--registry` (`<dir>/SKILL_REGISTRY.md`), `--write` | without `--write`, the table drifted; always, the file is missing, has no table, or cannot be read or written |
| `check <dir>` | `--registry`, `--index` | any error |

`check` errors on: a folder skill missing frontmatter `name` or `description`, a broken relative link, a registry without the table or drifted, a `skills.json` differing from a fresh index, and a line that looks like a private key or an AWS, OpenAI, GitHub or Slack token. Its one warning: a frontmatter `name` differing from the directory name. Registry and index are checked when present at the default path or passed.

`check` and `registry` print user text (names, link targets, paths, malformed rows, unknown options) matching those patterns as `<redacted>` (`redact`); a secret finding gives only `path:line`, the path masked too. `skills.json` keeps real paths ([How It Works](04-how-it-works.md)).

<sub>Sources: [main.rs:130-152](../crates/skrun/src/main.rs#L130-L152) · [main.rs:508-556](../crates/skrun/src/main.rs#L508-L556) · [main.rs:435-506](../crates/skrun/src/main.rs#L435-L506) · [catalog.rs:60-66](../crates/skrun/src/catalog.rs#L60-L66) · [catalog.rs:353-400](../crates/skrun/src/catalog.rs#L353-L400) · +4</sub>

## Environment and default paths

| name | effect |
|---|---|
| `SKRUN_SKILLS_DIR` | skills root used by `run --id`, `install-local`, `list`, `show` and the Python SDK |
| `HOME` | fallback root `$HOME/.skrun/skills`; the CLI and Rust callers fail with `HOME is not set` if neither is set |

The CLI computes this default eagerly (`root.unwrap_or(default_skills_dir()?)`), so with neither variable set, `run --id`, `install-local`, `list` and `show` fail with `HOME is not set` even when `--root` is given.

`build` of a `rust_binary` uses `<system temp dir>/skrun-skill-targets/<id>` as Cargo's target directory unless `--target-dir` is given, so by default skill builds write no Cargo target directory into the skill directory. The Python SDK resolves the same root itself (expanding `~`), and passes its `root=` argument through when you set one.

<sub>Sources: [runtime.rs:50-53](../crates/skrun/src/runtime.rs#L50-L53) · [runtime.rs:719-725](../crates/skrun/src/runtime.rs#L719-L725) · [runtime.rs:954-959](../crates/skrun/src/runtime.rs#L954-L959) · [skill.py:242-248](../python/skrun/skill.py#L242-L248) · [main.rs:357-365](../crates/skrun/src/main.rs#L357-L365) · +3</sub>

## artifact.json

`artifact.json` deserializes into `SkillArtifact`. Unknown keys are ignored.

| field | required | notes |
|---|---|---|
| `schema_version` | yes | must be `1` |
| `kind` | yes | `markdown`, `rust_binary`, `python_uv` |
| `id`, `name`, `version` | yes | strings; `id` is validated (below) |
| `executable` | no, default `true` | must be `false` for `markdown` |
| `entry` | for executable kinds | relative path to the binary or Python script |
| `protocol` | no | `transport`, `input`, `output`; default `stdio-json`, `single-json-value`, `single-json-value`. Only `transport` is checked. |
| `schema` | no | `input`, `output`: relative paths to JSON Schema files |
| `description`, `tags`, `suggested_tools`, `content`, `source_ref` | no | metadata; filled from `SKILL.md` when absent |
| `source` | no | `{"type": ..., "ref": ...}` provenance, e.g. `"type": "git"` |

The schema files are documentation for callers: for executable kinds the runtime checks that their paths are relative, but it never validates input or output against them.

<sub>Sources: [runtime.rs:63-123](../crates/skrun/src/runtime.rs#L63-L123) · [runtime.rs:252-254](../crates/skrun/src/runtime.rs#L252-L254) · [runtime.rs:214-238](../crates/skrun/src/runtime.rs#L214-L238)</sub>

## SKILL.md frontmatter

`SKILL.md` may start with a `---` block. skrun reads only these keys, with a minimal line parser rather than full YAML:

- `name`, `description`, `version`
- `source_ref` (alias `ref`)
- `tags`, `suggested_tools`: inline `[a, b]` / `a, b`, or a following `- item` list

Quotes around values are stripped; every other key (the examples' `id:` and `kind:` included) is ignored. The text after the block becomes `content`. For a directory with only `SKILL.md`, the id is the directory name, the name falls back to the first `# ` heading and then to the title-cased id, and the version defaults to `0.1.0`.

<sub>Sources: [runtime.rs:458-563](../crates/skrun/src/runtime.rs#L458-L563) · [runtime.rs:392-425](../crates/skrun/src/runtime.rs#L392-L425)</sub>

## Id and path rules

An id must be non-empty, contain only ASCII letters, digits, `-` and `_`, and start with a letter or digit. The same check applies to `--id` on `install-local`, so an installed directory name is always a safe single path segment.

For the executable kinds (`rust_binary`, `python_uv`), `entry`, `schema.input` and `schema.output` must be non-empty relative paths with no `..` or root components: the check is lexical, so a symlink inside the directory can still resolve outside it. A `markdown` artifact's `schema.*` paths are not checked.

<sub>Sources: [runtime.rs:789-826](../crates/skrun/src/runtime.rs#L789-L826) · [runtime.rs:206-241](../crates/skrun/src/runtime.rs#L206-L241)</sub>

## Python API

| call | returns |
|---|---|
| `skrun.skill(id_or_path, root=None)` | `SkillHandle` (loads and validates the artifact) |
| `SkillHandle.call(value=None, timeout=60.0)` | the skill's output `dict` |
| `skrun.list_skills(root=None)` | `list[SkillArtifact]` |
| `skrun.install_local_skill(source, root=None, *, skill_id=None, overwrite=False)` | `SkillHandle` looked up by the installed manifest's id |
| `skrun.build_skill(root, target_dir=None)` | `SkillArtifact` |
| `skrun.load_artifact(root)` | `SkillArtifact` |

`SkillRuntime(root=..., uv=...)` groups the same operations around one skills root. Every failure, including a missing native extension, raises `SkillRuntimeError`. Two details differ from the CLI: `timeout` is truncated to whole seconds, and the `uv` field on `SkillRuntime`/`SkillHandle` is stored but not passed to the native call, so Python skills always run the `uv` found on PATH.

Known issue: `install_local` returns `self.skill(artifact.id)`. When `skill_id=` renames a skill that has `artifact.json`, the copy keeps the manifest's original id, so the returned handle resolves `<root>/<original id>` instead of the installed copy, and fails or points at a different skill. Look the copy up yourself with `skrun.skill(skill_id, root=...)`.

<sub>Sources: [__init__.py:7-35](../python/skrun/__init__.py#L7-L35) · [skill.py:172-234](../python/skrun/skill.py#L172-L234) · [skill.py:251-292](../python/skrun/skill.py#L251-L292)</sub>
