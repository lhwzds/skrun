---
title: 风险规则
source: #847ca2bb1c59
---

`scan_skill` 把每个存储的文件按有损 UTF-8 读成文本；每条规则对每个文件报告第一处匹配，形式为 `path:line`。二进制文件和非常大的文件跳过模式规则，但不跳过密钥检查。

| 严重程度 | 规则 | 发现什么 |
|---|---|---|
| high | `remote_exec` | curl 或 wget 通过管道交给 shell 或 Python；`iex`、`Invoke-Expression`、`DownloadString` |
| high | `decode_exec` | 解码调用（`b64decode`、`fromhex`、`atob`、带 base64 的 `Buffer.from` 等）之后 160 个字符内出现 exec、eval、compile 或 sh |
| high | `reverse_shell`、`cred_theft`、`exfil_endpoint`、`persistence`、`av_evasion` | /dev/tcp 和 nc -e；SSH 密钥、AWS 凭据、钥匙串、浏览器 cookie、具名钱包；webhook 或粘贴服务以及指向公网 IPv4 主机的 URL；cron、LaunchAgents、shell rc 文件；绕过隔离属性、Gatekeeper、SIP 或 Defender |
| high | `binary` | 二进制扩展名或魔数（ELF、Mach-O、PE） |
| medium | `obfuscation` | 很长的 base64 或十六进制转义串，大量 `_0x` 标识符 |
| medium | `social_engineering` | 仅限 Markdown 和文本文件：复制/粘贴到终端，带密码的压缩包 |
| medium | `secret`、`symlink`、`shadows_stdlib` | [[reference]] 中的密钥模式；被跳过的符号链接；以危险模块命名的 Python 文件或包（见下文） |

已知缺陷：在 JavaScript 中，`Buffer.from(...)` 参数里的注释会让 `decode_exec` 漏报。
