---
id: sec33-reformat-rest-3
title: Reformat：剩下的换行和代码风格页签（第三批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec33-reformat-rest-3
pr: 245
claimed: 2026-11-08
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：

- Kotlin Spaces 页剩下的：In simple one line methods 和 lambda 花括号内的空格（`{x}` → `{ x }`，默认开），Range operator（`..`，默认关），Unary operator（默认关）。
- Java 的 Within 组（括号、方括号等，默认都关，现在固定去掉括号内的空格）。
- 值移到下一行后仍超长时的换行、Do not keep line breaks、Wrapping 页、Imports 页。

箭头的空格已在 sec33-reformat-rest-2（#243）做完，Java 的 Before left brace / In ternary operator 在 sec33-reformat-rest（#236）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-08 认领。这次只做 Kotlin Spaces 页剩下的几项（lambda / 单行方法花括号内的空格、Range、Unary），其余部分做完后另建任务文件。
- 2026-11-08 完成（#245）：`Spaces` 加了 `around_unary`、`around_range`、`simple_braces`（`values` 下标 44–46，`SPACES_LEN = 47`，旧设置按 `from_prefix` 补默认值），Kotlin 页的顺序照 IntelliJ 放在 Multiplicative 后和 Before lambda arrow 前。`normalize` 里：`{` 在同一行有匹配的 `}`（或后面有 lambda 的 `->`）时按 `simple_brace` 判断是不是 lambda / 函数体，再决定内侧空格，`braces` 栈记住这一行开的 `{`，到 `}` 时处理前面的空格；`'.'` 分支处理 `..` / `..<`；`unary` 判断前缀 `!`、正负号和前后缀 `++` / `--`（前面是操作数、`}`、`!!`、`++` 时不算前缀，避免 `} + x`、`}!!` 被当成一元）。坑：第一次跑 Signal 语料时 `}!!` 变成 `}! !`、`} + x` 变成 `} +x`，把 `}` 算作操作数后改动行数回到 16776。测试 `braces_range_and_unary_spaces`。剩下的 Java Within、Java Unary、换行和 Imports 页移到 sec33-reformat-rest-4。
