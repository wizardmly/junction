---
id: sec33-reformat-rest-15
title: Reformat：Wrapping and Braces 页剩下的选项（第十五批）
features: 第 33 节
status: open
owner:
branch:
pr:
claimed:
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 if / for / while 语句体的换行（Place on new line / Special else if treatment）、参数和局部变量注解、Kotlin 的注解换行。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 25 位，旧的照样读，缺的位用语言默认值）。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。注意这个测试不比第二遍：要查稳定，把输出目录当输入再跑一遍，和第一遍的输出 diff（见任务 reformat-second-pass-stability）。新选项打开时第二遍不能多出变化。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
