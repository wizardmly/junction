---
id: sec33-reformat-rest-10
title: Reformat：Wrapping and Braces 页剩下的选项（第十批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-10
pr: 263
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 if / for / while 语句体的换行和 Force braces、注解换行（类 / 方法 / 字段 / 参数）、枚举常量、二元表达式、三元表达式、Kotlin 的 Put left brace on new line。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261）。类型是 `domain::reformat::Wrapping` / `Braces`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（旧的照样读）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-09：这批做了 Java 的 Force braces（'if' ('else') / 'for' / 'while'）和 Kotlin 的 Put left brace on new line，其余拆到 sec33-reformat-rest-11；发现不带花括号的语句体不缩进，拆到 reformat-braceless-body-indent。
  - `domain::reformat::Force`（DoNotForce / WhenMultiline / Always），放在 `Braces.force: [Force; 3]`；`Braces::with_values` 改七个 bool 时保留 force。
  - `force_braces`（只 Java，在 `brace_layout` 前跑）：`head_end` 找 `if` / `else` / `else if` / `for` / `while` 头的右括号（`close_paren` 跳过字符串），`}` 开头的 `while` 是 do-while 不算；`simple` 判断语句体是一条以 `;` 结尾的简单语句（不是块、不是 if / for 等关键字语句）。Always 拆单行写法，WhenMultiline / Always 包下一行的语句体。
  - Kotlin 的 Put left brace on new line 复用 `Braces.other_next_line`（第 13 位）：`brace_layout` 的 Kotlin 分支认控制语句头（以 `)` 结尾或单个关键字）和 class / interface / object / fun / constructor / init 声明（顶层没有 `=`、没有 `->`）。
  - settings.conf `code_style_java_wrapping` 第 14–16 位；设置页 Java 三个下拉框，Kotlin 一个勾选框。
  - 回归：Signal Java 400 个改动行数 15764 不变，Kotlin 200 个输出与之前相同；选项全开时再格式化一遍不变。
