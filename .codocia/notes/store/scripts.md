---
title: How scripts and imports are read
order: 38
evidence:
  - crates/skrun/src/store.rs:1608-1652 #50b830178f3c
  - crates/skrun/src/catalog.rs:1335-1394 #8b56cf3da580
  - crates/skrun/src/catalog.rs:1638-1679 #f67c748aef52
  - crates/skrun/src/catalog.rs:1769-1815 #28509ea200df
  - crates/skrun/src/store.rs:1469-1540 #7d90f2a2eed9
  - crates/skrun/src/risk.rs:45-49 #7c17cab8d000
  - commit:ae10cbf
---

A file is a script by extension (`.py`, `.sh`, `.js`/`.mjs`, `.ts`), else by a `#!` line: `sh`, `bash`, `zsh`, `dash` mean shell, `python*` Python, `node`, `deno`, `bun` JavaScript, anything else `other`. Python imports count outside strings and comments, even after `;` or `:` (`if True: import x`), plus `__import__` and `importlib.import_module` with a string literal. Python that cannot be tokenised (unbalanced brackets, unterminated strings, bad indentation) is `unparsed`. Known gap: a triple-quoted module name inside `__import__` is missed.

An import is third-party unless it is in CPython 3.14's `sys.stdlib_module_names` or local: a `.py` file, or a folder holding one, beside the script or above it.
