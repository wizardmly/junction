---
id: sec33-reformat-rest-42
title: Reformat：Wrapping and Braces 页剩下的选项（第四十二批）
features: 第 33 节
status: done
owner: Claude Code / Jake
branch: task/sec33-reformat-rest-42
pr: 301
claimed: 2026-11-10
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 Java 的 Text blocks › Align when multiline（ALIGN_MULTILINE_TEXT_BLOCKS）、Switch expressions（SWITCH_EXPRESSIONS_WRAP）、Labeled statement（LABELED_STATEMENT_WRAP）、'case' statement on new line、Extends keyword 等，以及对照 IntelliJ 的 Wrapping and Braces 页还没有的其他项。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276），Java Keep when reformatting 的 Simple blocks / methods / lambdas / classes 和 Control statement in one line（#277），Special 'else if' treatment、'for()' statement 换行和 Align when multiline（#278），'try-with-resources' 和 for / try 的括号换行（#279），Java 'assert' statement、Java 方法调用 / 声明括号换行和 Kotlin Parameter annotations（#280），Kotlin 方法调用 / 声明括号换行和 Java Parenthesized expression（#281），Java 参数 / 实参的 Align when multiline 和 Record components（#282），Java Binary expressions 的 Align when multiline（#283），Ternary / Throws list / Extends list 的 Align when multiline（#284），Assignment / Chained method calls / Array initializer 的 Align when multiline（#285），Kotlin 'when' 分支对齐和多行分支后换行（#286），Java Deconstruction list（#287），Kotlin 七处 Use continuation indent（#288），Comment at first column、Indent 'case' branches 和 Kotlin context 参数续行（#289），Wrap first call 和 Group declarations 的字段 / 变量 / 赋值对齐（#290），Multiple expressions in one line 和 Wrap after modifier list（#291），Align simple methods in columns、Assignment sign on next line 和 Indent 'break' from 'case'（#292），Array initializer 的大括号换行和 Do not wrap after single annotation（#293），Java Builder methods（#294，名字列表在 settings.conf 的 `code_style_java_builders`），Keep builder methods indents（#295），Do not indent top level class members 和 Wrap trailing semicolon after call chain（#296），Multi-catch types（#297），In lambda declaration: Next line（#299）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 101 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。Kotlin 格式化器在 plugins/kotlin/formatter/minimal/src/org/jetbrains/kotlin/idea/formatter/（KotlinCommonBlock.kt、kotlinSpacingRules.kt、KotlinLanguageCodeStyleSettingsProvider.kt），测试数据在 plugins/kotlin/idea/tests/testData/formatter/；容器里可以从 raw.githubusercontent.com 下载。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-10：这批做了 Java 的 Text blocks › Align when multiline 和 'switch' statement › 'case' statement on new line。`Wrapping::align_text_blocks`（默认关，wrapping 下标 101）和 `case_new_line`（默认开，下标 102）。文本块：缩进器在 `starts_in == Raw` 的行用 `text_block_layout` 算出块的最后一行、公共缩进（结尾行算、空白行不算，照 Java 规范和 IntelliJ 的 `extractTextRangesFromLiteralText`）和新缩进（开头行缩进 + 续行缩进，对齐时是开头 `"""` 的列；用 tab 时不对齐），`text_block_line` 逐行重放；行内括号位置跟着平移再交给 `apply_brackets`。`scan` 在 Java 的 Raw 状态里跳过反斜杠转义（`\"""` 不再被当成结尾）。只在有 wrapping（即 Reformat Code）时做。case：`split_one_line_bodies` 多了 `case_line` 参数，`split_case_statement` 在标签的 `:` 后拆行，跳过 `::`、`->` 规则、`{` 块和注释。设置页加了两个复选框。测试 `lays_out_text_blocks_and_case_statements`（含 IntelliJ 的 testTextBlock、test_alignMultilineTextBlock）。语料：默认 5201 行（多出的 98 行都是 `case X: return y;` 拆成两行，IntelliJ 的 JavaSpacePropertyProcessor 在 CASE_STATEMENT_ON_NEW_LINE 开时强制换行），第二遍 0 行；两个选项反过来时 5103 行、第二遍 0 行（语料里没有文本块）。
