---
title: 参考
---

# 参考

<sub>[English](../03-reference.md) · **中文**</sub>

CLI、环境变量、文件格式和 Python API 的准确名称，均摘自代码。`skrun`、`skrun -h`、`skrun --help` 和 `skrun help` 都会打印用法说明。

<sub>来源: [main.rs:438-450](../../crates/skrun/src/main.rs#L438-L450)</sub>

## CLI 命令与参数

所有命令都在 `skrun skill` 下。位置参数 `<dir>` / `<id>` 可以出现在参数之间的任意位置。

| 命令 | 参数（默认值） |
|---|---|
| `new <dir>` | `--kind markdown\|guidance\|rust_binary\|python_uv`（必填）、`--id`（必填）、`--name`（id 转为首字母大写：`regex-finder` → `Regex Finder`）、`--version`（`0.1.0`） |
| `build <dir>` | `--profile`（`release`）、`--target-dir`、`--cargo`（`cargo`）、`--uv`（`uv`） |
| `run <dir>` 或 `run --id <id>` | `--input`（`{}`；必须是 JSON 对象）、`--timeout` 秒数（`60`）、`--uv`（`uv`）、`--root`（仅配合 `--id`） |
| `install-local <dir>` | `--root`、`--id`（安装时重命名）、`--overwrite` |
| `list` | `--root`、`--format text\|json`（`text`） |
| `show <id>` | `--root`、`--format text\|json`（`text`） |

`--root` 默认为 skill 根目录（见下文）。`list` 和 `show` 的文本输出是以制表符分隔的 `id kind version name`。给 `run` 同时传目录和 `--id` 会报错。`--cargo` 和 `--uv` 存在，但没有写进用法说明。出错时会向 stderr 打印 `Error: ...` 并以 1 退出。

<sub>来源: [main.rs:44-49](../../crates/skrun/src/main.rs#L44-L49) · [main.rs:215-244](../../crates/skrun/src/main.rs#L215-L244) · [main.rs:246-311](../../crates/skrun/src/main.rs#L246-L311) · [main.rs:313-374](../../crates/skrun/src/main.rs#L313-L374) · [main.rs:376-397](../../crates/skrun/src/main.rs#L376-L397) · +1</sub>

## 环境变量与默认路径

| 名称 | 作用 |
|---|---|
| `SKRUN_SKILLS_DIR` | `run --id`、`install-local`、`list`、`show` 和 Python SDK 使用的 skill 根目录 |
| `HOME` | 后备根目录 `$HOME/.skrun/skills`；两者都未设置时，CLI 和 Rust 调用方会以 `HOME is not set` 失败 |

CLI 会提前计算这个默认值（`root.unwrap_or(default_skills_dir()?)`），所以两个变量都未设置时，即使传了 `--root`，`run --id`、`install-local`、`list` 和 `show` 也会以 `HOME is not set` 失败。

`build` 构建 `rust_binary` 时，除非指定 `--target-dir`，否则使用 `<system temp dir>/skrun-skill-targets/<id>` 作为 Cargo 的 target 目录，因此 skill 构建永远不会在 skill 目录中写入 Cargo target 目录。Python SDK 自行解析同样的根目录（会展开 `~`），设置了 `root=` 参数时则直接传递该参数。

<sub>来源: [runtime.rs:50-53](../../crates/skrun/src/runtime.rs#L50-L53) · [runtime.rs:719-725](../../crates/skrun/src/runtime.rs#L719-L725) · [runtime.rs:946-951](../../crates/skrun/src/runtime.rs#L946-L951) · [skill.py:242-248](../../python/skrun/skill.py#L242-L248) · [main.rs:300-308](../../crates/skrun/src/main.rs#L300-L308) · +3</sub>

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

对于可执行类型（`rust_binary`、`python_uv`），`entry`、`schema.input` 和 `schema.output` 必须是非空的相对路径，且不能包含 `..` 或根路径成分：清单无法指向自身目录之外。`markdown` 制品的 `schema.*` 路径不做检查。

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

已知问题：`install_local` 返回 `self.skill(artifact.id)`。当用 `skill_id=` 重命名一个带 `artifact.json` 的 skill 时，副本保留的是清单中原来的 id，因此返回的 handle 解析到的是 `<root>/<original id>` 而不是安装的副本，结果要么失败，要么指向另一个 skill。请自己用 `skrun.skill(skill_id, root=...)` 查找副本。

<sub>来源: [__init__.py:7-35](../../python/skrun/__init__.py#L7-L35) · [skill.py:172-234](../../python/skrun/skill.py#L172-L234) · [skill.py:251-292](../../python/skrun/skill.py#L251-L292)</sub>
