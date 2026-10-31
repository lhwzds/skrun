---
title: Risk rules
order: 30
evidence:
  - crates/skrun/src/risk.rs:377-480 #7c03e8f776a1
  - crates/skrun/src/risk.rs:492-550 #a7c018a17683
  - crates/skrun/src/risk.rs:616-685 #5369f0b8eef3
  - crates/skrun/src/risk.rs:722-805 #7814b4769e00
  - crates/skrun/src/store.rs:1134-1218 #1c2289f0900e
  - crates/skrun/src/store.rs:1293-1333 #1b95e349e99c
  - crates/skrun/src/store.rs:1431-1446 #10a7cefcee8f
  - commit:7d7f4d4
---

`scan_skill` reads every stored file as lossy UTF-8 text; each rule reports its first match per file as `path:line`. Binary and very large files skip the pattern rules, not the secret check.

| severity | rule | finds |
|---|---|---|
| high | `remote_exec` | curl or wget piped into a shell or Python; `iex`, `Invoke-Expression`, `DownloadString` |
| high | `decode_exec` | a decode call (`b64decode`, `fromhex`, `atob`, `Buffer.from` with base64, …) followed within 160 characters by exec, eval, compile or sh |
| high | `reverse_shell`, `cred_theft`, `exfil_endpoint`, `persistence`, `av_evasion` | /dev/tcp and nc -e; SSH keys, AWS credentials, Keychain, browser cookies, named wallets; webhook or paste services and URLs to public IPv4 hosts; cron, LaunchAgents, shell rc files; quarantine, Gatekeeper, SIP or Defender bypass |
| high | `binary` | a binary extension or magic number (ELF, Mach-O, PE) |
| medium | `obfuscation` | long base64 or hex-escape runs, many `_0x` identifiers |
| medium | `social_engineering` | Markdown and text files only: copy/paste into a terminal, password-protected archives |
| medium | `secret`, `symlink`, `shadows_stdlib` | the secret patterns of [[reference]]; a skipped symlink; a Python file or package named after a risky module (below) |

Known gap: in JavaScript, a comment inside the `Buffer.from(...)` arguments hides a `decode_exec`.
