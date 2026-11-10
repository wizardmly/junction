---
id: sec33-reformat-rest-52
title: Reformat：Wrapping and Braces 页剩下的选项（第五十二批）
features: 第 33 节
status: done
owner: Claude Code / Jake
branch: task/sec33-reformat-rest-52
pr: 312
claimed: 2026-11-10
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 Enum field annotations（ENUM_FIELD_ANNOTATION_WRAP）、Record components 的 Annotations new line（ANNOTATION_NEW_LINE_IN_RECORD_COMPONENT）、Use flying geese braces、Javadoc 里无星号行补 ` * `、Kotlin 页还没有的项等，以及对照 IntelliJ 的 Wrapping and Braces 页还没有的其他项。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276），Java Keep when reformatting 的 Simple blocks / methods / lambdas / classes 和 Control statement in one line（#277），Special 'else if' treatment、'for()' statement 换行和 Align when multiline（#278），'try-with-resources' 和 for / try 的括号换行（#279），Java 'assert' statement、Java 方法调用 / 声明括号换行和 Kotlin Parameter annotations（#280），Kotlin 方法调用 / 声明括号换行和 Java Parenthesized expression（#281），Java 参数 / 实参的 Align when multiline 和 Record components（#282），Java Binary expressions 的 Align when multiline（#283），Ternary / Throws list / Extends list 的 Align when multiline（#284），Assignment / Chained method calls / Array initializer 的 Align when multiline（#285），Kotlin 'when' 分支对齐和多行分支后换行（#286），Java Deconstruction list（#287），Kotlin 七处 Use continuation indent（#288），Comment at first column、Indent 'case' branches 和 Kotlin context 参数续行（#289），Wrap first call 和 Group declarations 的字段 / 变量 / 赋值对齐（#290），Multiple expressions in one line 和 Wrap after modifier list（#291），Align simple methods in columns、Assignment sign on next line 和 Indent 'break' from 'case'（#292），Array initializer 的大括号换行和 Do not wrap after single annotation（#293），Java Builder methods（#294，名字列表在 settings.conf 的 `code_style_java_builders`），Keep builder methods indents（#295），Do not indent top level class members 和 Wrap trailing semicolon after call chain（#296），Multi-catch types（#297），In lambda declaration: Next line（#299），Text blocks 的 Align when multiline 和 'case' statement on new line（#301），Switch expressions 和 Extends keyword（#303），Labeled statement、Align parenthesised when multiline 和 Align throws keyword（#304），Ensure right margin is not exceeded 的行注释和字符串（#305），块注释和 Javadoc（#306），Parenthesized expression 的 Align when multiline（#307），Take priority over call chain wrapping（#308），Ensure right margin 对普通代码的强制断行（#309），Annotation parameters 的 Align when multiline 和括号换行（#310），New line when body is presented（#311）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 114 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。Kotlin 格式化器在 plugins/kotlin/formatter/minimal/src/org/jetbrains/kotlin/idea/formatter/（KotlinCommonBlock.kt、kotlinSpacingRules.kt、KotlinLanguageCodeStyleSettingsProvider.kt），测试数据在 plugins/kotlin/idea/tests/testData/formatter/；容器里可以从 raw.githubusercontent.com 下载。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-10：Java 的 Enum field annotations（ENUM_FIELD_ANNOTATION_WRAP，IntelliJ `JavaFormatterUtil` 里给 ENUM_CONSTANT 用的注解 Wrap；`Wrapping::enum_annotations`，settings.conf 第 115 位，AnnotationWrap 的下标，默认 Do not wrap）。`wrap_annotations` 的 `wraps` 改成 5 项（Kotlin 第 5 项是 Do not wrap），另用 `enums` 栈记每层括号是不是还没过第一个 `;` 的枚举体，行在里面且注解后是 `enum_constant`（名字后接 `,` `;` `(` `{` 或行尾）时用第 5 项。设置页 `annotation_options` 多 “Enum field annotations:”（只 Java）。测试 `wraps_enum_field_annotations`。Signal 语料没有带注解的枚举常量：默认和打开都是 5201 行、第二遍 0 行。剩下的放到 sec33-reformat-rest-53。
