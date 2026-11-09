---
id: reformat-braceless-body-indent
title: Reformat：不带花括号的 if / for / while 语句体缩进
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/reformat-braceless-body-indent
pr: 264
claimed: 2026-11-09
files: src/domain/reformat.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ 把 `if (x)`、`else`、`for (…)`、`while (…)` 下一行不带花括号的语句体缩进一级（Java 和 Kotlin 都是），Junction 的 Reformat Code 现在只按花括号算缩进，这种语句体保持原来的缩进，写歪了也不会改正。

注意：语句体后面的行要回到原来的缩进；嵌套的 `if (a)` / `if (b)` / `x();` 每层加一级；`else if` 链；Kotlin 的 `if (x) a else b` 表达式写在多行时。`domain/reformat.rs` 的测试 `force_braces_and_kotlin_left_brace` 里现在按“不缩进”写了预期，做完要改。用 Signal 语料回归（HANDOFF.md 6.4），改动行数不能变多。

## 进度

- 2026-11-09：做完。原来不只是不改缩进，而是把语句体缩进到和 `if` 一样（栈里没有花括号，缩进按外层块算）。
  - `statement_head` / `head_at_start`：一行的代码（字符串换成 `_`、去掉注释）是 `if (…)` / `else` / `else if (…)` / `for (…)` / `while (…)` / `do` 且到行尾结束；或者结尾是 Kotlin 的 `if (…)` 表达式（前面是空格、`(` 或 `=`）。`}` 开头的 `while` 是 do-while 结尾，不算——坑：Kotlin 的 `} while (x)` 没有 `;`，第一次跑语料时把后面一行多缩进了。
  - `reformat_with` 的 `Previous` 加 `head`：上一行是 head、括号都关了、深度相同、这行不是续行也不以 `{` 开头时，缩进 = 上一行 + 一级。只有注释的行也缩进，且不覆盖 `previous`，语句体还在下一行。
  - 没做：条件跨多行（`if (a &&` / `b)` 后面的语句体），head 的右括号在后面一行时认不出来。
  - 回归：Signal Java 改动行数 15764 → 15762，Kotlin 23752 → 23750，变化的行都和原文一致。
