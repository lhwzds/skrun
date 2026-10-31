---
title: 
order: 0
evidence:
  - crates/skrun/src/main.rs:676-771 #352f54f30902
  - crates/skrun/src/main.rs:858-923 #37e65bb8aeed
  - crates/skrun/src/store.rs:594-726 #828a7be44406
  - crates/skrun/src/store.rs:531-583 #fbb499f2fb78
---

`skrun catalog add` copies a skill from elsewhere (another repository, a downloaded zip) into a local store, and `inspect` shows what arrived. Neither executes, builds or imports anything from the skill, and neither reaches the network: the store is where you look at a skill before deciding to install it ([[how-it-works]]).

```bash
skrun catalog add <dir|zip> [--store <dir>] [--id <id>] [--from <provenance>] [--json]
skrun catalog inspect <id> [--store <dir>] [--json]
skrun catalog inspect --list [--store <dir>] [--json]
```

`add` prints `stored`, `already stored` or `already stored (record refreshed)` with the id, version hash, file and finding counts and the tier; `--json` prints the version record. `inspect` prints each version with its provenance, scripts, findings by severity and tier reasons; `--list` prints one line per id for its newest version (`--json` works there too, though the usage text omits it). Inspecting an id that was never added fails. Every printed string goes through `redact`, also for a record edited by hand.
