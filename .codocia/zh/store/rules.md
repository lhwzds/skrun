---
title: 风险规则
source: #ffe8a600dc76
---

`scan_skill` 把每个文件按有损 UTF-8 读取；每条规则对每个文件报告第一处匹配，形式为 `path:line`。二进制文件和超过 16 MB（`MAX_SCAN_BYTES`）的文件跳过这些规则，但不跳过密钥检查。

| 严重程度 | 规则 | 发现什么 |
|---|---|---|
| high | `remote_exec` | curl 或 wget 通过管道交给 shell 或 Python；`iex`、`Invoke-Expression`、`DownloadString` |
| high | `decode_exec` | 解码调用（`b64decode`、`fromhex`、`atob`、base64 的 `Buffer.from` 等）之后 160 个字符内出现 exec、eval、compile 或 sh |
| high | `reverse_shell`、`cred_theft`、`exfil_endpoint`、`persistence`、`av_evasion` | /dev/tcp、nc -e；SSH 密钥、AWS 凭据、钥匙串、浏览器 cookie、钱包；webhook 或粘贴服务、指向公网 IPv4 的 URL；cron、LaunchAgents、shell rc 文件；绕过隔离属性、Gatekeeper、SIP 或 Defender |
| high | `binary` | 二进制扩展名或魔数（ELF、Mach-O、PE） |
| medium | `obfuscation`、`social_engineering` | 很长的 base64 或十六进制转义串，大量 `_0x` 标识符；Markdown 和文本中诱导粘贴到终端或使用带密码压缩包的内容 |
| medium | `secret`、`symlink`、`unscanned`、`unparsed`、`shadows_stdlib` | 密钥；被跳过的符号链接；超过 16 MB 的文件；无法分词的 Python；与标准库模块同名、位于脚本所在文件夹或其上层的 `.py` 文件、无扩展名文件或 Python 文件夹 |

已知缺陷：JavaScript 中 `Buffer.from(...)` 参数里的注释会让 `decode_exec` 漏报。
