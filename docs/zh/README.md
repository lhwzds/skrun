---
title: skrun
---

# skrun

<sub>[English](../README.md) · **中文**</sub>

这份文档面向想把本地工具打包成可执行 skill、并在 agent 框架中调用它们的开发者。内容包括：skill 制品是什么；如何通过 CLI 或 Python 生成脚手架、构建、安装和运行 skill；清单字段、命令行参数和环境变量的准确名称；Rust 运行时如何强制执行 stdio JSON 契约；以及如何参与仓库本身的开发。README 负责简要介绍，这些页面则更深入一层，内容均依据代码编写。

## 目录

1. [概览](01-overview.md) — 三种 skill · 一个运行时，三个入口 · 为什么工作区这么小
2. [快速上手](02-getting-started.md) — 安装 · 创建、构建并运行一个 skill · 安装 skill 并从 Python 调用 · 试一个真实示例 · 从 agent 框架调用 skill
3. [参考](03-reference.md) — CLI 命令与参数 · Catalog 命令 · 环境变量与默认路径 · artifact.json · SKILL.md frontmatter · id 与路径规则 · Python API
4. [工作原理](04-how-it-works.md) — 加载 skill 目录 · 一次调用如何执行 · 哪些情况算失败 · 构建 · 安装与列出 · Python 边界 · 为什么 catalog 的输出是确定的
5. [参与开发](05-contributing.md) — 本地开发流程 · 冒烟测试验证了什么 · CI 与发布 · 新增 skill kind 或命令

## 附录

- [代码结构](structure.md) — 从代码生成：功能区、依赖、讲解页

<sub>生成自 https://github.com/lhwzds/skrun @ `dec4d0c294`</sub>
