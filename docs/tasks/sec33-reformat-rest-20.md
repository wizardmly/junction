---
id: sec33-reformat-rest-20
title: Reformat：Wrapping and Braces 页剩下的选项（第二十批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-20
pr: 277
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 if / for / while 语句体的换行（Place on new line / Special else if treatment）、Kotlin 的参数注解（PARAMETER_ANNOTATION_WRAP）。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 31 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：Java 的 Keep when reformatting：Simple blocks / methods / lambdas / classes in one line、Control statement in one line。类型 `domain::reformat::KeepOneLine`（`Wrapping::keep_one_line`，settings.conf 第 31–35 位，缺的位当勾选）。实现是 `split_one_line_bodies`：反复调 `split_bodies_once` 直到不再变化，每行找第一个在同一行开合、按设置不保留的 `{…}`（`split_one_line_body` 用 `declaration_braces` 判类体，`->` 判 lambda，类体里 `member_kind` 是 Method 判方法，其余代码块；数组初始化、`switch` 跳过），`split_body_at` 按顶层 `;` 和结束的 `}`（后面不是 else / catch / finally / while）拆语句；`split_control_statement` 拆 `if` / `for` / `while` / `else` 的语句体。缩进交给后面的缩进器。设置页在 Line breaks 下加五个勾选框（只 Java）。
- 默认全部保留（和 IntelliJ 不同：IntelliJ 只保留控制语句），理由同注解换行：Signal 里有 42 个空方法 `{}`、175 处 `if (…) return;`，按 IntelliJ 默认会大面积改动。
- 回归：Signal 语料 Java 5107、Kotlin 24702 不变；五项全关多改 638 行，加上实参 / 形参 Chop 时第二遍也是 0 行。测试 `splits_simple_bodies_when_not_kept`。
- 剩下的（for 语句的换行和对齐、Special else if treatment、Kotlin 参数注解）挪到 sec33-reformat-rest-21。
