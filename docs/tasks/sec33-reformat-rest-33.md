---
id: sec33-reformat-rest-33
title: Reformat：Wrapping and Braces 页剩下的选项（第三十三批）
features: 第 33 节
status: done
owner: Claude Code / Jake
branch: task/sec33-reformat-rest-33
pr: 290
claimed: 2026-11-10
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 Java 的 Group declarations（ALIGN_GROUP_FIELD_DECLARATIONS、ALIGN_CONSECUTIVE_VARIABLE_DECLARATIONS 等）、Keep when reformatting › Multiple expressions in one line（KEEP_MULTIPLE_EXPRESSIONS_IN_ONE_LINE）、Modifier list › Wrap after modifier list、Chained method calls › Wrap first call（WRAP_FIRST_METHOD_IN_CALL_CHAIN）/ Builder methods、Text blocks，Kotlin 的 Wrap first call，以及对照 IntelliJ 的 Wrapping and Braces 页还没有的其他项。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276），Java Keep when reformatting 的 Simple blocks / methods / lambdas / classes 和 Control statement in one line（#277），Special 'else if' treatment、'for()' statement 换行和 Align when multiline（#278），'try-with-resources' 和 for / try 的括号换行（#279），Java 'assert' statement、Java 方法调用 / 声明括号换行和 Kotlin Parameter annotations（#280），Kotlin 方法调用 / 声明括号换行和 Java Parenthesized expression（#281），Java 参数 / 实参的 Align when multiline 和 Record components（#282），Java Binary expressions 的 Align when multiline（#283），Ternary / Throws list / Extends list 的 Align when multiline（#284），Assignment / Chained method calls / Array initializer 的 Align when multiline（#285），Kotlin 'when' 分支对齐和多行分支后换行（#286），Java Deconstruction list（#287），Kotlin 七处 Use continuation indent（#288），Comment at first column、Indent 'case' branches 和 Kotlin context 参数续行（#289）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 82 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。Kotlin 格式化器在 plugins/kotlin/formatter/minimal/src/org/jetbrains/kotlin/idea/formatter/（KotlinCommonBlock.kt、kotlinSpacingRules.kt、KotlinLanguageCodeStyleSettingsProvider.kt），测试数据在 plugins/kotlin/idea/tests/testData/formatter/；容器里可以从 raw.githubusercontent.com 下载。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-10：`Wrapping::wrap_first_call`（WRAP_FIRST_METHOD_IN_CALL_CHAIN，默认关，Java 和 Kotlin 都用）和 `group_align: [bool; 3]`（ALIGN_GROUP_FIELD_DECLARATIONS、ALIGN_CONSECUTIVE_VARIABLE_DECLARATIONS、ALIGN_CONSECUTIVE_ASSIGNMENTS，默认关），settings.conf wrapping 第 82–85 位（现在 86 位）。`chain_break` 多了 `first` 参数：打开时链里第一个调用的 `.` 也能当断点，但要等后面再出现一个调用才算（照 ChainMethodCallsBlockBuilder.canWrap）。Group declarations 是缩进之后的一遍 `group_declarations`：`grouped_line` 认出单行字段（类体里，`declaration_braces` 判断）、局部变量和赋值语句（`+=` 之类也算进赋值组但不对齐），按列补空格；字段组照 IntelliJ 的 SubsequentFieldAligner 对齐类型、名字和 `=`，注释行不断开，有修饰符和没修饰符的分开（IntelliJ 避免长缩进）；变量组照 SubsequentVariablesAlignerConfigurations 对齐名字和 `=`，赋值组对齐 `=`，注释也断开。
- Signal 语料：默认 0 行变化（Java 5103、Kotlin 25404）；全打开（Java 再把 Chained method calls 设成 Wrap if long）时第二遍 0 行，Java 和原文不同的行反而从 5103 降到 3319（Signal 自己就是对齐成列的风格）。
- 没做：字段上一行单独写的注解会断开字段组（IntelliJ 会接着对齐）；`int a, b;` 这种一行多个变量不参加对齐；Align simple methods in columns、Multiple expressions in one line、Modifier list、Builder methods、Text blocks 留给 sec33-reformat-rest-34。
