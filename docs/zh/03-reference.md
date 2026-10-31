---
title: 参考
---

# 参考

<sub>[English](../03-reference.md) · **中文**</sub>

CLI、环境变量、文件格式和 Python API 的准确名称，均摘自代码。`skrun`、`skrun -h`、`skrun --help` 和 `skrun help` 都会打印用法说明。

<sub>来源: [main.rs:337-340](../../crates/skrun/src/main.rs#L337-L340) · [main.rs:1004-1023](../../crates/skrun/src/main.rs#L1004-L1023)</sub>

## CLI 命令与参数

skill 命令都在 `skrun skill` 下；`skrun catalog` 见下文。位置参数 `<dir>` / `<id>` 可以出现在参数之间的任意位置。

| 命令 | 参数（默认值） |
|---|---|
| `new <dir>` | `--kind markdown\|guidance\|rust_binary\|python_uv`（必填）、`--id`（必填）、`--name`（id 转为首字母大写：`regex-finder` → `Regex Finder`）、`--version`（`0.1.0`） |
| `build <dir>` | `--profile`（`release`）、`--target-dir`、`--cargo`（`cargo`）、`--uv`（`uv`） |
| `run <dir>` 或 `run --id <id>` | `--input`（`{}`；必须是 JSON 对象）、`--timeout` 秒数（`60`）、`--uv`（`uv`）、`--root`（仅配合 `--id`） |
| `install-local <dir>` | `--root`、`--id`（安装时重命名）、`--overwrite` |
| `list` | `--root`、`--format text\|json`（`text`） |
| `show <id>` | `--root`、`--format text\|json`（`text`） |

`--root` 默认为 skill 根目录（见下文）。`list` 和 `show` 打印以制表符分隔的 `id kind version name`。`run` 不接受同时给出目录和 `--id`。`--cargo` 和 `--uv` 没有写进用法说明。出错时向 stderr 打印 `Error: ...` 并以 1 退出，用法错误（未知、缺失或无效的参数）则以 2 退出；看起来像密钥的回显参数和路径显示为 `<redacted>`。

<sub>来源: [main.rs:49-66](../../crates/skrun/src/main.rs#L49-L66) · [main.rs:364-395](../../crates/skrun/src/main.rs#L364-L395) · [main.rs:397-468](../../crates/skrun/src/main.rs#L397-L468) · [main.rs:470-537](../../crates/skrun/src/main.rs#L470-L537) · [main.rs:939-968](../../crates/skrun/src/main.rs#L939-L968) · +1</sub>

## Catalog 命令

`skrun catalog` 处理的是一个指导性 skill 目录：含 `SKILL.md` 的非隐藏子目录，以及除 `SKILL_REGISTRY.md`、`README.md`、`README_CN.md`、`AGENTS.md` 和 `CLAUDE.md` 之外的顶层 `*.md`。其中 `add` 和 `inspect` 处理的是存储（[存储与风险等级](05-store.md)）。

| 命令 | 参数（默认值） | 何时以 1 退出 |
|---|---|---|
| `index <dir>` | `--out <file>`（stdout） | 目录无法读取 |
| `registry <dir>` | `--registry`（`<dir>/SKILL_REGISTRY.md`）、`--write` | 不带 `--write` 时：表格有偏差；任何模式下：文件不存在、没有表格，或无法读写 |
| `check <dir>` | `--registry`、`--index` | 有任何错误 |
| `search <dir> <query>...` | `--kind folder\|flat`、`--has-scripts`、`--limit`（`10`）、`--json` | 没有匹配（`--json` 打印 `[]`） |

`check` 报为错误的有：缺少 frontmatter `name` 或 `description` 的文件夹 skill、失效的相对链接、缺失或有偏差的注册表表格、过期的 `skills.json`，以及含密钥的行。唯一的警告是 frontmatter `name` 与目录名不同。

密钥指私钥块、以 `AKIA`、`sk-`（含 `sk-proj-`、`sk-ant-`）、`gh[pousr]_`、`github_pat_`、`glpat-`、`xox[baprs]-` 或 `AIza` 开头的令牌，以及 `password: …` 这类通用赋值（[存储与风险等级](05-store.md)）。`check`、`registry` 和 `search` 打印的用户文本若匹配，会显示为 `<redacted>`（`redact`）；密钥发现只给出 `path:line`。`skills.json` 保留真实路径（[工作原理](04-how-it-works.md)）。

<sub>来源: [main.rs:173-211](../../crates/skrun/src/main.rs#L173-L211) · [main.rs:783-856](../../crates/skrun/src/main.rs#L783-L856) · [main.rs:539-771](../../crates/skrun/src/main.rs#L539-L771) · [catalog.rs:66-72](../../crates/skrun/src/catalog.rs#L66-L72) · [catalog.rs:373-420](../../crates/skrun/src/catalog.rs#L373-L420) · +6</sub>

## 环境变量与默认路径

| 名称 | 作用 |
|---|---|
| `SKRUN_SKILLS_DIR` | `run --id`、`install-local`、`list`、`show` 和 Python SDK 使用的 skill 根目录 |
| `HOME` | 后备根目录 `$HOME/.skrun/skills`；两者都未设置时，CLI 和 Rust 调用方会以 `HOME is not set` 失败 |
| `SKRUN_STORE` | 未给 `--store` 时 `catalog add` 和 `inspect` 使用的存储，否则为 `$HOME/.skrun/store`（[存储与风险等级](05-store.md)） |

CLI 会提前计算这个默认值（`root.unwrap_or(default_skills_dir()?)`），所以两个变量都未设置时，即使传了 `--root`，`run --id`、`install-local`、`list` 和 `show` 也会以 `HOME is not set` 失败。

`build` 构建 `rust_binary` 时，除非指定 `--target-dir`，否则使用 `<system temp dir>/skrun-skill-targets/<id>` 作为 Cargo 的 target 目录，因此 skill 构建不会在 skill 目录中留下 Cargo target 目录。Python SDK 自行解析同样的根目录（会展开 `~`），设置了 `root=` 参数时则直接传递该参数。

<sub>来源: [runtime.rs:50-53](../../crates/skrun/src/runtime.rs#L50-L53) · [runtime.rs:719-725](../../crates/skrun/src/runtime.rs#L719-L725) · [runtime.rs:954-959](../../crates/skrun/src/runtime.rs#L954-L959) · [skill.py:242-248](../../python/skrun/skill.py#L242-L248) · [main.rs:455-463](../../crates/skrun/src/main.rs#L455-L463) · +4</sub>

## artifact.json

`artifact.json` 会反序列化为 `SkillArtifact`，未知的键会被忽略。

| 字段 | 是否必填 | 说明 |
|---|---|---|
| `schema_version` | 是 | 必须为 `1` |
| `kind` | 是 | `markdown`、`rust_binary`、`python_uv` |
| `id`, `name`, `version` | 是 | 字符串；`id` 会被校验（见下文） |
| `executable` | 否，默认 `true` | `markdown` 必须为 `false` |
| `entry` | 可执行类型必填 | 指向二进制或 Python 脚本的相对路径 |
| `protocol` | 否 | `transport`、`input`、`output`；默认分别为 `stdio-json`、`single-json-value`、`single-json-value`。只检查 `transport`。 |
| `schema` | 否 | `input`、`output`：指向 JSON Schema 文件的相对路径 |
| `description`, `tags`, `suggested_tools`, `content`, `source_ref` | 否 | 元数据；缺省时从 `SKILL.md` 填充 |
| `source` | 否 | `{"type": ..., "ref": ...}` 来源信息，例如 `"type": "git"` |

schema 文件是写给调用方看的文档：对可执行类型，运行时会检查它们的路径是否为相对路径，但从不按 schema 校验输入或输出。

<sub>来源: [runtime.rs:63-123](../../crates/skrun/src/runtime.rs#L63-L123) · [runtime.rs:252-254](../../crates/skrun/src/runtime.rs#L252-L254) · [runtime.rs:214-238](../../crates/skrun/src/runtime.rs#L214-L238)</sub>

## SKILL.md frontmatter

`SKILL.md` 可以以一个 `---` 块开头。skrun 只读取下列键，用的是一个极简的逐行解析器，而不是完整的 YAML 解析：

- `name`、`description`、`version`
- `source_ref`（别名 `ref`）
- `tags`、`suggested_tools`：行内写法 `[a, b]` / `a, b`，或在后续行使用 `- item` 列表

值两侧的引号会被去掉；其他所有键（包括示例中的 `id:` 和 `kind:`）都会被忽略。块之后的文本成为 `content`。对于只有 `SKILL.md` 的目录，id 就是目录名，name 依次回退到第一个 `# ` 标题和首字母大写的 id，version 默认为 `0.1.0`。

<sub>来源: [runtime.rs:458-563](../../crates/skrun/src/runtime.rs#L458-L563) · [runtime.rs:392-425](../../crates/skrun/src/runtime.rs#L392-L425)</sub>

## id 与路径规则

id 不能为空，只能包含 ASCII 字母、数字、`-` 和 `_`，并且必须以字母或数字开头。`install-local` 的 `--id` 也做同样的检查，所以安装后的目录名始终是一个安全的单级路径段。

对于可执行类型（`rust_binary`、`python_uv`），`entry`、`schema.input` 和 `schema.output` 必须是非空的相对路径，且不能包含 `..` 或根路径成分：这只是对路径文本的检查，目录内的符号链接仍可能解析到目录之外。`markdown` 制品的 `schema.*` 路径不做检查。

<sub>来源: [runtime.rs:789-826](../../crates/skrun/src/runtime.rs#L789-L826) · [runtime.rs:206-241](../../crates/skrun/src/runtime.rs#L206-L241)</sub>

## Python API

| 调用 | 返回值 |
|---|---|
| `skrun.skill(id_or_path, root=None)` | `SkillHandle`（加载并校验制品） |
| `SkillHandle.call(value=None, timeout=60.0)` | skill 输出的 `dict` |
| `skrun.list_skills(root=None)` | `list[SkillArtifact]` |
| `skrun.install_local_skill(source, root=None, *, skill_id=None, overwrite=False)` | 按已安装清单中的 id 查找到的 `SkillHandle` |
| `skrun.build_skill(root, target_dir=None)` | `SkillArtifact` |
| `skrun.load_artifact(root)` | `SkillArtifact` |

`SkillRuntime(root=..., uv=...)` 把同样的操作组织在同一个 skill 根目录下。所有失败（包括缺少原生扩展）都会抛出 `SkillRuntimeError`。有两处与 CLI 不同：`timeout` 会被截断为整秒；`SkillRuntime`/`SkillHandle` 上的 `uv` 字段只是存着，并不会传给原生调用，所以 Python skill 总是运行 PATH 中找到的 `uv`。

已知问题：`install_local` 返回 `self.skill(artifact.id)`。当用 `skill_id=` 重命名一个带 `artifact.json` 的 skill 时，副本保留清单中的 id，因此 handle 解析到 `<root>/<original id>`，而不是副本。请用 `skrun.skill(skill_id, root=...)` 查找它。

<sub>来源: [__init__.py:7-35](../../python/skrun/__init__.py#L7-L35) · [skill.py:172-234](../../python/skrun/skill.py#L172-L234) · [skill.py:251-292](../../python/skrun/skill.py#L251-L292)</sub>
