---
title: skrun
repo: ..
commit: 7d7f4d4fa46bb3906972ebe2d6f07225309e3e13
remote: https://github.com/lhwzds/skrun
lang: en
max_pages: 8
max_page_words: 900
max_note_words: 180
pages:
  - overview: Overview
  - getting-started: Getting Started
  - reference: Reference
  - how-it-works: How It Works
  - store: Store and Risk Tiers
  - contributing: Contributing
ignore:
  - docs/**
  - python/tests/**
  - **/tests/**
  - Cargo.lock
  - .github/dependabot.yml
  - examples/skills/cdp-browser/src/**
  - .gitattributes
  - .gitignore
  - LICENSE
langs:
  - zh
translations:
  - README_CN.md: README.md #b8637eee1a78
---

These docs explain skrun for developers who want to package local tools as executable skills and call them from an agent framework. They cover what a skill artifact is, how to scaffold, build, install and run one from the CLI or Python, the exact manifest fields, flags and environment variables, how the Rust runtime enforces the stdio JSON contract, and how to work on the repository itself. The README gives the elevator pitch; these pages go one level deeper and are written from the code.
