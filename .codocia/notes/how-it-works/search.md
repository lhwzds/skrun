---
title: How search ranks
order: 95
evidence:
  - crates/skrun/src/catalog.rs:717-741 #a45679550625
  - crates/skrun/src/catalog.rs:788-864 #05eb50c65038
  - crates/skrun/src/catalog.rs:904-958 #f074536d4f90
  - crates/skrun/src/catalog.rs:1016-1054 #e535bc5f5d54
  - crates/skrun/src/catalog.rs:78-90 #b20478e49bbc
---

`search_skills` lowercases each whitespace-separated term and matches it as a substring, so Chinese needs no segmentation. Every term must hit a field (AND) and adds the weight of each field it hits: id 5, frontmatter name 4, registry title 4, registry summary, description and each tag 3, entrypoint body 1. Results sort by score, then id; `--has-scripts` drops flat skills. A result shows the registry title (else the name) and the registry summary (else the description's first sentence: up to `. `, `? `, `! `, `。`, `？`, `！` or a newline, ≤120 characters), redacted before the cut so a secret never prints in part.
