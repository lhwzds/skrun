---
title: 
order: 0
evidence:
  - crates/skrun/src/main.rs:676-771 #352f54f30902
  - crates/skrun/src/main.rs:858-923 #37e65bb8aeed
  - crates/skrun/src/store.rs:646-785 #2db5db47228d
  - crates/skrun/src/store.rs:583-635 #fbb499f2fb78
---

`skrun catalog add` copies a skill from elsewhere (another repository, a downloaded zip) into a local store; `inspect` shows what arrived. Neither executes, builds or imports anything from the skill, nor reaches the network: the store is where you look before installing ([[how-it-works]]).

```bash
skrun catalog add <dir|zip> [--store <dir>] [--id <id>] [--from <provenance>] [--json]
skrun catalog inspect <id> [--store <dir>] [--json]
skrun catalog inspect --list [--store <dir>] [--json]
```

`add` prints `stored`, `already stored` or `already stored (record refreshed)` with id, version hash, file and finding counts and tier; `--json` prints the version record. `inspect` prints each version's provenance, scripts, findings by severity and tier reasons; `--list` prints each id's newest version on one line. Every printed string goes through `redact`, even from a hand-edited record.
