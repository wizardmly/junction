---
id: sec33-reformat-rest-40
title: Reformat：Wrapping and Braces 页剩下的选项（第四十批）
features: 第 33 节
status: done
owner: Claude Code / Jake
branch: task/sec33-reformat-rest-40
pr: 297
claimed: 2026-11-10
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 Java 的 Text blocks › Align when multiline（ALIGN_MULTILINE_TEXT_BLOCKS）、Lambda braces（LAMBDA_BRACE_STYLE）、Switch expressions（SWITCH_EXPRESSIONS_WRAP）、Multi-catch types（MULTI_CATCH_TYPES_WRAP / ALIGN_TYPES_IN_MULTI_CATCH）、Labeled statement（LABELED_STATEMENT_WRAP）、'case' statement on new line、Extends keyword 等，以及对照 IntelliJ 的 Wrapping and Braces 页还没有的其他项。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276），Java Keep when reformatting 的 Simple blocks / methods / lambdas / classes 和 Control statement in one line（#277），Special 'else if' treatment、'for()' statement 换行和 Align when multiline（#278），'try-with-resources' 和 for / try 的括号换行（#279），Java 'assert' statement、Java 方法调用 / 声明括号换行和 Kotlin Parameter annotations（#280），Kotlin 方法调用 / 声明括号换行和 Java Parenthesized expression（#281），Java 参数 / 实参的 Align when multiline 和 Record components（#282），Java Binary expressions 的 Align when multiline（#283），Ternary / Throws list / Extends list 的 Align when multiline（#284），Assignment / Chained method calls / Array initializer 的 Align when multiline（#285），Kotlin 'when' 分支对齐和多行分支后换行（#286），Java Deconstruction list（#287），Kotlin 七处 Use continuation indent（#288），Comment at first column、Indent 'case' branches 和 Kotlin context 参数续行（#289），Wrap first call 和 Group declarations 的字段 / 变量 / 赋值对齐（#290），Multiple expressions in one line 和 Wrap after modifier list（#291），Align simple methods in columns、Assignment sign on next line 和 Indent 'break' from 'case'（#292），Array initializer 的大括号换行和 Do not wrap after single annotation（#293），Java Builder methods（#294，名字列表在 settings.conf 的 `code_style_java_builders`），Keep builder methods indents（#295），Do not indent top level class members 和 Wrap trailing semicolon after call chain（#296）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 98 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。Kotlin 格式化器在 plugins/kotlin/formatter/minimal/src/org/jetbrains/kotlin/idea/formatter/（KotlinCommonBlock.kt、kotlinSpacingRules.kt、KotlinLanguageCodeStyleSettingsProvider.kt），测试数据在 plugins/kotlin/idea/tests/testData/formatter/；容器里可以从 raw.githubusercontent.com 下载。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-10：这批只做 Java 的 Multi-catch types。`Wrapping::catch_wrap`（默认 Wrap if long）/ `align_catch`（默认开），settings.conf wrapping 下标 98、99（旧文件缺位时用默认）。复用 `wrap_for_statements`：关键字是 `catch` 时在括号里的 `|` 后断开（`;` 不算），`keyword_open` 对 `catch` 允许前面有 `}`；缩进器里 `keyword_paren(.., "catch")` 在 `multi_catch` 行上提供对齐列。设置页加了 Wrap 下拉框和对齐复选框。测试 `wraps_multi_catch_types`。Labeled statement 没做：IntelliJ 的 CommonCodeStyleSettings 里找不到 LABELED_STATEMENT_WRAP 的默认值定义，留给后面查清楚再做。语料：Signal 没有超长 multi-catch，默认和强制 Chop 都 0 行。
