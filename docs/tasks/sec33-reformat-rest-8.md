---
id: sec33-reformat-rest-8
title: Reformat：Do not keep line breaks 和 Wrapping and Braces 页的其他选项（第八批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-8
pr: 259
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：值移到下一行后仍超长时的换行；Keep when reformatting › Line breaks 关闭时合并多余的换行；Wrapping and Braces 页的其他选项（大括号位置、if / for / try 等语句的换行、注解、枚举常量、二元表达式等）。

已有的五项（实参、形参、链式调用、父类列表、赋值）在 sec33-reformat-rest-7（#256）做完，类型是 `domain::reformat::Wrapping`，新选项接着往里加。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-09：做了 Keep when reformatting › Line breaks；值移过去仍超长时保持不换（与 IntelliJ 对照一致，见 FEATURES.md）；页上其他选项拆到 sec33-reformat-rest-9。
  - `Wrapping.keep_line_breaks`（JAVA / KOTLIN 都是 true）；`reformat_with` 开头算出语言的 `wrapping`，关掉时先跑 `join_line_breaks(text, kotlin, only, margin)` 再照常格式化，`only` 按合掉的行数缩短。
  - `join_line_breaks`：第一遍记下带尾逗号的 `(` / `[`；第二遍逐行判断能否并到上一行（行首状态是代码、两行都无注释、上一行不以 `;` `{` `}` `->` 结尾，且在没有尾逗号的括号里、或上一行以二元运算符 / `=` 结尾、或本行以 `.` `?.` `?:` `&&` `||` 开头）；连续能并的一段整段并成一行，放不进右边界就整段不动。上一行的代码里字符串换成 `_`（坑：只去掉字符串时 `val A = "x"` 看起来以 `=` 结尾，会把两条语句并在一起）。
  - settings.conf 第 6 位，设置页勾选框 “Keep when reformatting: Line breaks”。
  - 回归：临时把默认改成关，Signal Java 400 个、Kotlin 200 个文件格式化稳定，抽查合并结果正常。
