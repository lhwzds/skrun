---
title: skrun
---

# skrun

<sub>**English** · [中文](zh/README.md)</sub>

These docs explain skrun for developers who want to package local tools as executable skills and call them from an agent framework. They cover what a skill artifact is, how to scaffold, build, install and run one from the CLI or Python, the exact manifest fields, flags and environment variables, how the Rust runtime enforces the stdio JSON contract, and how to work on the repository itself. The README gives the elevator pitch; these pages go one level deeper and are written from the code.

## Contents

1. [Overview](01-overview.md) — Three kinds of skill · One runtime, three front doors · Why the workspace is this small
2. [Getting Started](02-getting-started.md) — Install · Create, build and run a skill · Install it and call it from Python · Try a real example · Call a skill from an agent framework
3. [Reference](03-reference.md) — CLI commands and flags · Catalog commands · Environment and default paths · artifact.json · SKILL.md frontmatter · Id and path rules · Python API
4. [How It Works](04-how-it-works.md) — Loading a skill directory · How a call runs · What counts as failure · Builds · Install and list · The Python boundary · Why catalog output is deterministic
5. [Contributing](05-contributing.md) — Local development loop · What the smoke test proves · CI and releases · Adding a skill kind or command

## Appendix

- [Code structure](structure.md) — generated from the code: areas, dependencies, pages

<sub>Generated from https://github.com/lhwzds/skrun @ `46fffa3970`</sub>
