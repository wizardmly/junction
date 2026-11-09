---
id: sec33-reformat-rest-12
title: Reformat：Wrapping and Braces 页剩下的选项（第十二批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-12
pr: 268
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如注解换行（类 / 方法 / 字段 / 参数；IntelliJ 类和方法注解默认 Wrap always，直接照默认会改动很多现有代码，先看 Signal 语料再决定默认）、枚举常量、if / for / while 语句体的换行（Place on new line）、Kotlin 的 Elvis expressions；还有条件跨多行（`if (a &&` / `b)`）时下一行不带花括号的语句体缩进（reformat-braceless-body-indent 没做的部分）。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 20 位，旧的照样读）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-09：这批做了 Java 的 Class / Method / Field annotations 和条件跨多行时的语句体缩进，其余拆到 sec33-reformat-rest-13。
  - 注解：`domain::reformat::AnnotationWrap`（DoNotWrap / WrapIfLong / ChopIfLong / WrapAlways），`Wrapping.annotations: [AnnotationWrap; 3]`。`wrap_annotations` 是 Java 的预处理（在 `force_braces` 后、`brace_layout` 前）：用 `declaration_braces` 跟踪括号栈，只处理栈顶是类体（或顶层）的行；`leading_annotations` 拆出行首注解（括号里的字符串跳过，`@interface` 不算）；剩下部分用 `member_kind` 分类，以 `,` 结尾但 `declaration_paren` 认得出的是多行方法头。坑：第一次没把 `(` / `[` 压栈，多行形参列表里的 `@NonNull Locale locale)` 被当成字段拆开了。
  - 默认 Do not wrap：IntelliJ 默认 Wrap always，Signal Java 里有 82 处同一行的注解声明，照默认会全拆。
  - 新加 `Wrapping::with_values`（只换五个 Wrap，其他保留），设置页 Wrap 下拉框改用它。settings.conf 第 21–23 位。
  - 语句体缩进：`opens_head` 认出没在本行闭合的 `if (` / `for (` / `while (`，记下深度和缩进（`split_head`），闭合的那行以 `)` 结尾时把 `Previous.head`（现在是 `Option<usize>`，存语句体相对的缩进）设成 head 首行的缩进。
  - 回归：Signal Java 15746、Kotlin 23750 不变；三项注解都设 Wrap always 时 Java 15900，多出的全是拆开的注解，稳定。
