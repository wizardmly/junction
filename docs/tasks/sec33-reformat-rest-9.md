---
id: sec33-reformat-rest-9
title: Reformat：Wrapping and Braces 页的其他选项（第九批）
features: 第 33 节
status: open
owner:
branch:
pr:
claimed:
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如大括号位置（End of line / Next line）、if / for / while / try 语句的换行和 Force braces、注解换行、枚举常量、二元表达式、三元表达式、Kotlin 的 when / 表达式体等。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（sec33-reformat-rest-7，#256）和 Keep line breaks（sec33-reformat-rest-8，#259），类型是 `domain::reformat::Wrapping`，新选项接着往里加，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（旧的照样读）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
