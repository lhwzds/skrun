---
title: Risk rules
order: 30
evidence:
  - crates/skrun/src/risk.rs:379-482 #7c03e8f776a1
  - crates/skrun/src/risk.rs:494-552 #a7c018a17683
  - crates/skrun/src/risk.rs:37-43 #9e3c06fef8d0
  - crates/skrun/src/risk.rs:619-686 #55e556025f37
  - crates/skrun/src/risk.rs:725-808 #7814b4769e00
  - crates/skrun/src/store.rs:1354-1467 #3087ab70789a
  - crates/skrun/src/store.rs:1542-1606 #71806812f453
  - crates/skrun/src/risk.rs:877-928 #c62e06f77de4
  - crates/skrun/src/store.rs:1816-1831 #10a7cefcee8f
  - commit:7d7f4d4
  - commit:ae10cbf
---

`scan_skill` reads each file as lossy UTF-8; a rule reports its first match per file as `path:line`. Binary files and files over 16 MB (`MAX_SCAN_BYTES`) skip these rules, not the secret check.

| severity | rule | finds |
|---|---|---|
| high | `remote_exec` | curl or wget piped into a shell or Python; `iex`, `Invoke-Expression`, `DownloadString` |
| high | `decode_exec` | a decode call (`b64decode`, `fromhex`, `atob`, base64 `Buffer.from`, …) within 160 characters before exec, eval, compile or sh |
| high | `reverse_shell`, `cred_theft`, `exfil_endpoint`, `persistence`, `av_evasion` | /dev/tcp, nc -e; SSH keys, AWS credentials, Keychain, browser cookies, wallets; webhook or paste services, public IPv4 URLs; cron, LaunchAgents, shell rc files; quarantine, Gatekeeper, SIP or Defender bypass |
| high | `binary` | a binary extension or magic number (ELF, Mach-O, PE) |
| medium | `obfuscation`, `social_engineering` | long base64 or hex-escape runs, many `_0x` identifiers; in Markdown and text, paste-into-terminal or password-protected archive lures |
| medium | `secret`, `symlink`, `unscanned`, `unparsed`, `shadows_stdlib` | a secret; a skipped symlink; a file over 16 MB; untokenisable Python; a `.py`, extensionless file or Python folder named after a stdlib module beside or above a script |

Known gap: a JavaScript comment in `Buffer.from(...)` arguments hides a `decode_exec`.
