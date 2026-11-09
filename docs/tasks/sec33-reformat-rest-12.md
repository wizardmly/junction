---
id: sec33-reformat-rest-12
title: Reformat：Wrapping and Braces 页剩下的选项（第十二批）
features: 第 33 节
status: open
owner:
branch:
pr:
claimed:
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如注解换行（类 / 方法 / 字段 / 参数；IntelliJ 类和方法注解默认 Wrap always，直接照默认会改动很多现有代码，先看 Signal 语料再决定默认）、枚举常量、if / for / while 语句体的换行（Place on new line）、Kotlin 的 Elvis expressions；还有条件跨多行（`if (a &&` / `b)`）时下一行不带花括号的语句体缩进（reformat-braceless-body-indent 没做的部分）。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 20 位，旧的照样读）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
