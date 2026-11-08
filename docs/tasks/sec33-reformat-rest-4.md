---
id: sec33-reformat-rest-4
title: Reformat：剩下的换行和代码风格页签（第四批）
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

- Java 的 Within 组（括号、方括号等，默认都关，现在固定去掉括号内的空格）和 Java 的 Around operators › Unary operator（默认关）。
- 值移到下一行后仍超长时的换行、Do not keep line breaks、Wrapping 页、Imports 页。

Kotlin 的 In simple one line methods、Range、Unary 已在 sec33-reformat-rest-3（#245）做完，`Spaces` 的新项照它的写法加在 `values` 末尾。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
