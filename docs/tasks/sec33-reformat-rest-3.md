---
id: sec33-reformat-rest-3
title: Reformat：剩下的换行和代码风格页签（第三批）
features: 第 33 节
status: open
owner:
branch:
pr:
claimed:
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：

- Kotlin Spaces 页剩下的：In simple one line methods 和 lambda 花括号内的空格（`{x}` → `{ x }`，默认开），Range operator（`..`，默认关），Unary operator（默认关）。
- Java 的 Within 组（括号、方括号等，默认都关，现在固定去掉括号内的空格）。
- 值移到下一行后仍超长时的换行、Do not keep line breaks、Wrapping 页、Imports 页。

箭头的空格已在 sec33-reformat-rest-2（#243）做完，Java 的 Before left brace / In ternary operator 在 sec33-reformat-rest（#236）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
