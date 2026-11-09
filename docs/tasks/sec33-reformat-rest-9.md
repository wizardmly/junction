---
id: sec33-reformat-rest-9
title: Reformat：Wrapping and Braces 页的其他选项（第九批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-9
pr: 261
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如大括号位置（End of line / Next line）、if / for / while / try 语句的换行和 Force braces、注解换行、枚举常量、二元表达式、三元表达式、Kotlin 的 when / 表达式体等。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（sec33-reformat-rest-7，#256）和 Keep line breaks（sec33-reformat-rest-8，#259），类型是 `domain::reformat::Wrapping`，新选项接着往里加，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（旧的照样读）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-09：这批做了大括号位置（类 / 方法 / 其他）和 'else' / 'while' / 'catch' / 'finally' on new line，其余拆到 sec33-reformat-rest-10。
  - `domain::reformat::Braces`（七个 bool，`values` / `from_values` / `LABELS`），放在 `Wrapping.braces`；`reformat_with` 先跑两遍 `brace_layout`（拆 `} else {` 后 `else {` 那行的 `{` 可能还要移），再跑 Keep line breaks 的合并。
  - `brace_layout`：用 `{` 栈记哪个块是 `do` 开的，`while` 只在关 do 块的 `}` 后算；`else` 后面是 `->`（Kotlin when）不算——坑：第一次跑 Kotlin 语料时把 `}` 和 `else ->` 并成了一行。`{` 的种类：首个词是控制语句为“其他”，`declaration_words` 是 class / interface / record / enum 为类，`declaration_paren` 认出的方法头为方法；含 `->`、`new `、以 `=` / `]` 结尾的不动。
  - settings.conf `code_style_<lang>_wrapping` 第 7–13 位；设置页 Java 七个、Kotlin 三个勾选框。
  - 回归：Signal Java 400 个改动行数 15764 不变，Kotlin 200 个输出与之前完全相同。
