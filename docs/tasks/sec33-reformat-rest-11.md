---
id: sec33-reformat-rest-11
title: Reformat：Wrapping and Braces 页剩下的选项（第十一批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-11
pr: 266
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如注解换行（类 / 方法 / 字段 / 参数；IntelliJ 类和方法注解默认 Wrap always，直接照默认会改动很多现有代码，先看 Signal 语料再决定默认）、枚举常量、二元表达式、三元表达式、if / for / while 语句体的换行（Place on new line）。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（旧的照样读）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-09：这批做了 Java 的 Binary expressions 和 Ternary operation（各一个 Wrap 和“符号在下一行”），其余拆到 sec33-reformat-rest-12。
  - `domain::reformat::Operations`（`binary` / `binary_sign_next` / `ternary` / `ternary_signs_next`，`NONE`），放在 `Wrapping.ops`；`Wrapping::any` 也看它。
  - `operation_break`（`chop` 里列表都不用换时、在 chain / assignment 之前，只 Java）：扫出两边有空格的 `|| && + - * / % ? :`（`:` 要有未配对的 `?`，排除 label 和 for-each），选层数最少、第一个括号里、优先级最低的那组切开。选项关着的运算符不算。
  - 先修了缩进（不然换出来的行第二遍会变）：`reformat_with` 里上一行开了括号且以运算符结尾（`if (a &&`）时这行记 Binary / Operator 但不再加续行缩进（`in_opened`），之后同类的行对齐；`java_ternary_end` 认行尾 ` ?` / ` :`；行首 `+ ` `- ` `* ` `/ ` `% ` 记 Operator。Signal Java 改动行数 15762 → 15746，全是和原文一致的对齐。
  - 设置页：`operation_options` 两个下拉框两个勾选框；修好 Wrap 下拉框用 `Wrapping::from_values` 重建、丢掉 keep_line_breaks / braces / force 的问题。settings.conf 第 17–20 位。
  - 回归：默认 Java 15746、Kotlin 23750；Chop / 行尾符号和 Wrap if long / 下一行符号两种全开都稳定。
  - 更正（2026-11-09，sec33-reformat-rest-14 时）：上面的“稳定”没真比过第二遍（corpus 测试只写结果）。真比后两种全开时 Signal Java 各有 35 行第二遍还会变（链式调用和二元运算续行的缩进），记在任务 reformat-second-pass-stability。
