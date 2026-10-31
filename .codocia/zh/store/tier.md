---
title: 建议的等级
source: #c293ee5e4a88
---

`suggest_tier` 把第一条匹配的行记为 `tier`，并附上 `tier_reasons`：

| 等级 | 条件 | 适合运行在 |
|---|---|---|
| `L3` | high 发现项、`unscanned` 的脚本，或遮蔽危险模块（`subprocess`、`socket`、`threading` 等）的文件 | microVM |
| `L2` | 非 Python 脚本（shell、JavaScript、TypeScript、`other`）、第三方或危险导入、`unscanned` 或 `unparsed` 的文件、任何 `shadows_stdlib` | 容器 |
| `L1` | 文件系统导入（`os`、`pathlib`、`shutil`、`io` 等）或 `open(` 调用 | 沙箱 |
| `L0` | 只做计算的 Python 脚本 | 普通进程 |
| `none` | 没有脚本 | 无需运行 |

附带一个 `subprocess.py` 只会提高等级，绝不会降低。等级只是记录下来的建议：skrun 目前没有沙箱，也没有任何地方读回这个等级。
