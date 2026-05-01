---
id: cdp-browser
name: CDP Browser
kind: skill_binary
---

# CDP Browser

Binary skill that controls Chrome through the Chrome DevTools Protocol.

The binary accepts either command-line arguments or a JSON request from stdin. It prints one JSON response to stdout.

## Actions

- `describe`: Return skill metadata without side effects.
- `launch`: Start Chrome with a CDP port.
- `status`: Check whether CDP is reachable.
- `open`: Open a URL in a new tab.
- `list`: List current tabs.
- `eval`: Evaluate JavaScript in the selected tab.
- `click`: Click an element by CSS selector.
- `type`: Type text into an element by CSS selector.
- `screenshot`: Capture the selected tab to a PNG file.

## Input

```json
{
  "action": "open",
  "port": 9222,
  "url": "https://example.com"
}
```

```json
{
  "action": "type",
  "port": 9222,
  "selector": "input[name='q']",
  "text": "RestFlow"
}
```

## Output

```json
{
  "ok": true,
  "action": "open",
  "data": {
    "target_id": "ABC",
    "url": "https://example.com/"
  },
  "error": null
}
```

## CLI Examples

```bash
cargo run -- launch --port 9222
cargo run -- open --port 9222 --url https://example.com
cargo run -- eval --port 9222 --expression "document.title"
cargo run -- screenshot --port 9222 --path /tmp/cdp.png
```
