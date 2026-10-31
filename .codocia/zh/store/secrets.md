---
title: 什么算密钥
source: #ad7c2b58dfbd
---

除了 [[reference]] 中的令牌，`line_has_secret` 还会标记一种赋值（`=` 或 `:`，带引号的键也算）：键中以完整单词出现 `password`、`passwd`、`pwd`、`secret`、`token`、`api_key`、`client_secret`、access key 或 private key（`x-api-key` 算，作为最后一段的 `POSTGRES_PASSWORD` 也算；`max_tokens` 不算），且值看起来是真的：至少六个非空白字符，并且不是 `$` 或 `%` 变量、表达式、类型注解或占位符（`<token>`、`changeme`、`YOUR_KEY` 等）。发现项只给出 `path:line`；`redact` 把这类字符串打印为 `<redacted>`。已知缺陷：以占位符单词开头、后跟真实密钥的 YAML 值会漏报。
