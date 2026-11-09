---
id: sec33-reformat-rest-19
title: Reformat：Wrapping and Braces 页剩下的选项（第十九批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-19
pr: 276
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 if / for / while 语句体的换行（Place on new line / Special else if treatment）、Kotlin 的注解换行。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 31 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：Kotlin 的 Class / Method / Field / Local variable annotations。没加新字段，用 `Wrapping::annotations` 和 `local_annotations`（Kotlin 那行 settings.conf 原来就存，第 21–23、26 位）。`wrap_annotations` 加了 `kotlin` 参数：`member_kind(rest, true)` 的 Class / Object → 类，Fun / Constructor → 方法，Property → 字段（代码块里 → 局部变量）。`leading_annotations` 认 Kotlin 的使用处目标（`@get:Rule`）。设置页 `annotation_options` 加 `lang` 参数，Kotlin 页只显示四项（没有 Enum constants 和 Parameter annotations）。
- 回归：Signal 语料 Java 5107、Kotlin 24702 不变；Kotlin 四项都设成 Wrap always / Wrap if long / Chop down if long 时第二遍都是 0 行（Signal 的 Kotlin 注解基本已单独成行，Wrap always 只多拆 1 处）。测试 `wraps_kotlin_annotations`。
- 没做：Kotlin 的 Parameter annotations（Kotlin 参数不对齐、续行一级，断开后怎么缩进要先查 IntelliJ 的 Kotlin 格式化器）、if / for / while 语句体换行，挪到 sec33-reformat-rest-20。
