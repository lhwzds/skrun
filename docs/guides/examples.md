---
title: Examples
description: Example executable skills shipped with skrun.
covers:
  - examples/skills/*/src/*.rs
  - examples/skills/*/skill.py
  - python/tests/test_skill_runtime.py
  - python/tests/test_package_script.py
---

# Examples

The skrun repository includes small examples under `examples/skills`.

## regex-finder

`regex-finder` is a Rust binary skill that evaluates regular expressions over
provided text.

```bash
skrun skill build examples/skills/regex-finder
skrun skill run \
  --input '{"action":"match","input":{"pattern":"\\d+","text":"abc 123"}}' \
  examples/skills/regex-finder
```

Expected output:

```json
{
  "ok": true,
  "action": "match",
  "data": {
    "matched": true
  },
  "error": null
}
```

## cdp-browser

`cdp-browser` is a browser-control skill shape built around a strict stdio-json
command protocol.

```bash
skrun skill build examples/skills/cdp-browser
skrun skill run --input '{"action":"describe"}' examples/skills/cdp-browser
```

The example demonstrates how an existing tool can become a skill by adding:

- `artifact.json`
- `SKILL.md`
- input and output JSON schemas
- a single stdin/stdout executable entrypoint

## Release Assets

Example skill release assets are published from tags such as:

```bash
git tag regex-finder@0.1.2
git tag cdp-browser@0.1.3
git push origin --tags
```

The release workflow builds the binary, runs its smoke test, packages the skill
directory, and uploads a `.tar.gz` asset to GitHub Releases.

Example skill docs should stay aligned with the checked-in Rust and Python
skills, their smoke tests, and the release packaging path.
