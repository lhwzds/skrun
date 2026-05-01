---
id: regex-finder
name: Regex Finder
kind: skill_binary
---

# Regex Finder

Binary skill that reads JSON from stdin, evaluates a regex using Rust's `regex` crate, and prints one structured JSON response to stdout.

## Input

```json
{
  "action": "match",
  "input": {
    "pattern": "foo.*bar",
    "text": "foo test bar"
  }
}
```

## Output

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
