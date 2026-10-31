# skrun

English | [中文](README_CN.md)

skrun is a Python-first executable skill runtime that lets agent frameworks
discover, build, install, and call local skills.

Documentation is maintained in [`docs/`](./docs/README.md). Releases and source
are available from the [GitHub repository](https://github.com/lhwzds/skrun).

## Principles

- Use short module names.
- Give each module a narrow responsibility.
- Keep UI interaction out of runtime semantics.
- Keep durable storage out of the executable skill runtime.
- Keep Python bindings at the same module boundary as Rust APIs.
- Support external agent frameworks instead of replacing their main agent loop.
- Make Rust binary skills and uv-backed Python skills first-class.

## Modules

```text
crates/skrun  runtime: artifacts, scaffold, build, install, and run; plus the skrun CLI
crates/py     skrun-native PyO3 module used by the Python package
python/skrun  Python SDK over skrun-native
```

## Skills

skrun supports guidance-only Markdown skills and executable artifacts:

```text
markdown     SKILL.md-only guidance skill, listed and shown but not executed
rust_binary  Cargo-built executable skill, called with stdin/stdout JSON
python_uv    uv-managed Python project, called with stdin/stdout JSON
```

Executable skill directories contain an `artifact.json` manifest:

```json
{
  "schema_version": 1,
  "kind": "rust_binary",
  "id": "regex-finder",
  "name": "Regex Finder",
  "version": "0.1.0",
  "entry": "bin/release/regex-finder",
  "protocol": {
    "transport": "stdio-json",
    "input": "single-json-value",
    "output": "single-json-value"
  },
  "schema": {
    "input": "schema/input.json",
    "output": "schema/output.json"
  }
}
```

The initial protocol is intentionally strict:

- input is one JSON value written to stdin
- output is one JSON object written to stdout
- stderr is diagnostics
- non-zero exit is failure
- streaming is not part of the first executable skill contract

## Python Package Loop

The PyPI distribution and Python import are both `skrun`. The Python package is
backed by the Rust `skrun-native` PyO3 module. Use the packaging helper so local
installs and release wheels use the same settings:

```bash
python3 -m pip install maturin
python3 python/scripts/package.py develop
python3 python/scripts/package.py smoke
python3 python/scripts/package.py build
python3 python/scripts/package.py sdist
```

The helper sets `PYO3_PYTHON` and keeps Cargo artifacts under
`/tmp/skrun-python-target` by default, which avoids executing Rust build
artifacts from an external macOS volume.

## Python Skill API

The high-level Python API calls local executable skill artifacts through the
same Rust `runtime` crate used by the CLI:

```python
import skrun

result = skrun.skill("/path/to/skill").call({"action": "match", "input": {"pattern": "TODO", "text": "TODO: ship it"}})
```

By default, skill IDs resolve under `~/.skrun/skills`, or under the directory
set by `SKRUN_SKILLS_DIR`. The Python package requires the
`skrun_native` PyO3 extension for executable skill operations; it does not
maintain a separate subprocess runtime fallback.

## CLI Skill Loop

The minimal CLI can scaffold guidance or executable skills:

```bash
cargo run -p skrun -- \
  skill new --kind markdown --id team /tmp/team

cargo run -p skrun -- \
  skill new --kind rust_binary --id rust-echo /tmp/rust-echo

cargo run -p skrun -- \
  skill build /tmp/rust-echo

cargo run -p skrun -- \
  skill run --input '{"ok":true}' /tmp/rust-echo

cargo run -p skrun -- \
  skill install-local --root /tmp/skrun-skills --overwrite /tmp/rust-echo

cargo run -p skrun -- \
  skill list --root /tmp/skrun-skills --format json

cargo run -p skrun -- \
  skill show --root /tmp/skrun-skills --format json rust-echo
```

Example skills live under `examples/skills`:

```text
rust-echo      minimal Rust stdio-json echo skill
python-echo    minimal uv-backed Python stdio-json echo skill
regex-finder   Rust binary regex matching skill
cdp-browser    CDP browser-control binary skill
```

The richer binary examples show how existing stdin/stdout tools become skrun
artifacts by adding `artifact.json` and JSON schema files.

## Skills Repository Catalog

`skrun catalog` maintains a directory of guidance skills: subdirectories with
`SKILL.md` plus top-level `*.md` files (except `SKILL_REGISTRY.md`, READMEs,
`AGENTS.md`, `CLAUDE.md`). It never runs skills, calls git or the network.

```bash
# write a deterministic skills.json index (stdout without --out)
cargo run -p skrun -- catalog index skills --out skills/skills.json

# report SKILL_REGISTRY.md drift; exits 1 when out of sync or the table is missing
cargo run -p skrun -- catalog registry skills

# drop stale/duplicate rows, append rows for unlisted skills; exits 0 once in sync
cargo run -p skrun -- catalog registry skills --write

# frontmatter, links, registry, stale index, possible secrets; exits 1 on errors
cargo run -p skrun -- catalog check skills

# rank skills by keyword (every term must match); exits 1 when nothing matches
cargo run -p skrun -- catalog search skills regex --limit 5

# copy an external skill into the read-only store and scan it; never runs it
cargo run -p skrun -- catalog add ./downloaded-skill --from https://example.com/skill.zip

# print the stored versions, findings and suggested tier of one skill
cargo run -p skrun -- catalog inspect downloaded-skill
```

## Agent Framework Examples

skrun does not replace the main agent framework. Wrap executable skills from
the framework you already use:

```python
import skrun


def regex_finder_tool(arguments: dict) -> dict:
    return skrun.skill("regex-finder").call(arguments)
```

Dependency-light examples live under `examples/frameworks` for LangChain,
LangGraph, PydanticAI, and OpenAI Agents-style tool bodies.

## Non-Goals For This Slice

- Do not implement a main agent framework.
- Do not implement a remote marketplace UI.
- Do not implement streaming executable skills.
