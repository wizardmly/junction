---
id: reformat-braceless-body-indent
title: Reformat：不带花括号的 if / for / while 语句体缩进
features: 第 33 节
status: open
owner:
branch:
pr:
claimed:
files: src/domain/reformat.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ 把 `if (x)`、`else`、`for (…)`、`while (…)` 下一行不带花括号的语句体缩进一级（Java 和 Kotlin 都是），Junction 的 Reformat Code 现在只按花括号算缩进，这种语句体保持原来的缩进，写歪了也不会改正。

注意：语句体后面的行要回到原来的缩进；嵌套的 `if (a)` / `if (b)` / `x();` 每层加一级；`else if` 链；Kotlin 的 `if (x) a else b` 表达式写在多行时。`domain/reformat.rs` 的测试 `force_braces_and_kotlin_left_brace` 里现在按“不缩进”写了预期，做完要改。用 Signal 语料回归（HANDOFF.md 6.4），改动行数不能变多。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
