---
title: skrun
repo: ..
commit: dec4d0c294f0d4ad678b429eeb1bca03cc9caa60
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
  - README_CN.md: README.md #7e8b7ee4f191
---

These docs explain skrun for developers who want to package local tools as executable skills and call them from an agent framework. They cover what a skill artifact is, how to scaffold, build, install and run one from the CLI or Python, the exact manifest fields, flags and environment variables, how the Rust runtime enforces the stdio JSON contract, and how to work on the repository itself. The README gives the elevator pitch; these pages go one level deeper and are written from the code.
