---
title: Reference
---

# Reference

<sub>**English** · [中文](zh/03-reference.md)</sub>

Exact names for the CLI, environment, file formats and Python API, copied from the code. `skrun`, `skrun -h`, `skrun --help` and `skrun help` print the usage text.

<sub>Sources: [main.rs:337-340](../crates/skrun/src/main.rs#L337-L340) · [main.rs:1004-1023](../crates/skrun/src/main.rs#L1004-L1023)</sub>

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

`--root` defaults to the skills root (below). `list` and `show` print tab-separated `id kind version name`. `run` rejects a directory plus `--id`. `--cargo` and `--uv` are missing from the usage text. Errors print `Error: ...` to stderr and exit 1, or 2 for a usage error (unknown, missing or invalid argument); echoed arguments and paths that look like a secret print as `<redacted>`.

<sub>Sources: [main.rs:49-66](../crates/skrun/src/main.rs#L49-L66) · [main.rs:364-395](../crates/skrun/src/main.rs#L364-L395) · [main.rs:397-468](../crates/skrun/src/main.rs#L397-L468) · [main.rs:470-537](../crates/skrun/src/main.rs#L470-L537) · [main.rs:939-968](../crates/skrun/src/main.rs#L939-L968) · +1</sub>

## Catalog commands

`skrun catalog` works on a directory of guidance skills: non-hidden subdirectories with a `SKILL.md`, plus top-level `*.md` except `SKILL_REGISTRY.md`, `README.md`, `README_CN.md`, `AGENTS.md` and `CLAUDE.md`. Its `add` and `inspect` work on the store instead ([Store and Risk Tiers](05-store.md)).

| command | flags (default) | exits 1 when |
|---|---|---|
| `index <dir>` | `--out <file>` (stdout) | the directory cannot be read |
| `registry <dir>` | `--registry` (`<dir>/SKILL_REGISTRY.md`), `--write` | without `--write`, the table drifted; always, the file is missing, has no table, or cannot be read or written |
| `check <dir>` | `--registry`, `--index` | any error |
| `search <dir> <query>...` | `--kind folder\|flat`, `--has-scripts`, `--limit` (`10`), `--json` | nothing matches (`--json` prints `[]`) |

`check` errors on: a folder skill missing frontmatter `name` or `description`, a broken relative link, a missing or drifted registry table, a stale `skills.json`, and a line holding a secret. Its one warning: a frontmatter `name` differing from the directory name.

Secrets are private-key blocks and tokens starting `AKIA`, `sk-` (incl. `sk-proj-`, `sk-ant-`), `gh[pousr]_`, `github_pat_`, `glpat-`, `xox[baprs]-` or `AIza`. `check`, `registry` and `search` print matching user text as `<redacted>` (`redact`); a secret finding gives only `path:line`. `skills.json` keeps real paths ([How It Works](04-how-it-works.md)).

<sub>Sources: [main.rs:173-211](../crates/skrun/src/main.rs#L173-L211) · [main.rs:783-856](../crates/skrun/src/main.rs#L783-L856) · [main.rs:539-771](../crates/skrun/src/main.rs#L539-L771) · [catalog.rs:63-69](../crates/skrun/src/catalog.rs#L63-L69) · [catalog.rs:370-417](../crates/skrun/src/catalog.rs#L370-L417) · +6</sub>

## Environment and default paths

| name | effect |
|---|---|
| `SKRUN_SKILLS_DIR` | skills root used by `run --id`, `install-local`, `list`, `show` and the Python SDK |
| `HOME` | fallback root `$HOME/.skrun/skills`; the CLI and Rust callers fail with `HOME is not set` if neither is set |
| `SKRUN_STORE` | store of `catalog add` and `inspect` when `--store` is absent, else `$HOME/.skrun/store` ([Store and Risk Tiers](05-store.md)) |

The CLI computes this default eagerly (`root.unwrap_or(default_skills_dir()?)`), so with neither variable set, `run --id`, `install-local`, `list` and `show` fail with `HOME is not set` even when `--root` is given.

`build` of a `rust_binary` uses `<system temp dir>/skrun-skill-targets/<id>` as Cargo's target directory unless `--target-dir` is given, so skill builds leave no Cargo target directory in the skill directory. The Python SDK resolves the same root itself (expanding `~`), and passes its `root=` argument through when you set one.

<sub>Sources: [runtime.rs:50-53](../crates/skrun/src/runtime.rs#L50-L53) · [runtime.rs:719-725](../crates/skrun/src/runtime.rs#L719-L725) · [runtime.rs:954-959](../crates/skrun/src/runtime.rs#L954-L959) · [skill.py:242-248](../python/skrun/skill.py#L242-L248) · [main.rs:455-463](../crates/skrun/src/main.rs#L455-L463) · +4</sub>

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

Known issue: `install_local` returns `self.skill(artifact.id)`. When `skill_id=` renames a skill that has `artifact.json`, the copy keeps the manifest's id, so the handle resolves `<root>/<original id>`, not the copy. Look it up with `skrun.skill(skill_id, root=...)`.

<sub>Sources: [__init__.py:7-35](../python/skrun/__init__.py#L7-L35) · [skill.py:172-234](../python/skrun/skill.py#L172-L234) · [skill.py:251-292](../python/skrun/skill.py#L251-L292)</sub>
