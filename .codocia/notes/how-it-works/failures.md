---
title: What counts as failure
order: 30
evidence:
  - crates/skrun/src/runtime.rs:1027-1066 #eeffbe7324a6
  - examples/skills/regex-finder/src/main.rs:27-35 #85e7354271ac
---

- **Timeout**: after `timeout` (60 s by default) the process is killed and the call fails with `skill process timed out after Ns`.
- **Non-zero exit**: the call fails with the exit code and trimmed stderr. Stdout is discarded, so a skill that prints a structured error body and exits 1 (as `regex-finder` does) loses that body; put the message on stderr too, or exit 0 with `"ok": false`.
- **Bad output**: stdout must be exactly one JSON value and it must be an object; arrays, scalars or several JSON values are errors (one object spread over many lines is fine).

Stdout and stderr are only collected after the process exits. Untested, from reading the code: a skill that writes more than the OS pipe buffer before exiting would block until the timeout. To be safe, keep outputs small or write large results to a file and return its path.
