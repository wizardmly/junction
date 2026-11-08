---
id: sec33-reformat-rest-2
title: Reformat：剩下的换行和代码风格页签（第二批）
features: 第 33 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec33-reformat-rest-2
pr:
claimed: 2026-11-08
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：值移到下一行后仍超长时的换行、Do not keep line breaks、Code Style 设置页剩下的（Spaces 的 Within、Kotlin 的 Before left brace 等、Wrapping、Imports）。Java 的 Before left brace / In ternary operator 已在 sec33-reformat-rest（#236）做完。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归，改动行数不能变多。

## 进度

这次先做 Code Style › Spaces 页剩下的选项：Kotlin 的 Before left brace、Java / Kotlin 的 Within（括号、方括号等，默认关）。换行（Wrapping）、Do not keep line breaks、Imports 页留给后面。

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
