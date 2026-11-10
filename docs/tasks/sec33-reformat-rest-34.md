---
id: sec33-reformat-rest-34
title: Reformat：Wrapping and Braces 页剩下的选项（第三十四批）
features: 第 33 节
status: done
owner: Claude Code / Jake
branch: task/sec33-reformat-rest-34
pr: 291
claimed: 2026-11-10
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 Java 的 Group declarations › Align simple methods in columns（ALIGN_SUBSEQUENT_SIMPLE_METHODS）、Keep when reformatting › Multiple expressions in one line（KEEP_MULTIPLE_EXPRESSIONS_IN_ONE_LINE）、Modifier list › Wrap after modifier list、Chained method calls › Builder methods（BUILDER_METHODS，是个名字列表，settings.conf 要另加一个键）、Text blocks，以及对照 IntelliJ 的 Wrapping and Braces 页还没有的其他项。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276），Java Keep when reformatting 的 Simple blocks / methods / lambdas / classes 和 Control statement in one line（#277），Special 'else if' treatment、'for()' statement 换行和 Align when multiline（#278），'try-with-resources' 和 for / try 的括号换行（#279），Java 'assert' statement、Java 方法调用 / 声明括号换行和 Kotlin Parameter annotations（#280），Kotlin 方法调用 / 声明括号换行和 Java Parenthesized expression（#281），Java 参数 / 实参的 Align when multiline 和 Record components（#282），Java Binary expressions 的 Align when multiline（#283），Ternary / Throws list / Extends list 的 Align when multiline（#284），Assignment / Chained method calls / Array initializer 的 Align when multiline（#285），Kotlin 'when' 分支对齐和多行分支后换行（#286），Java Deconstruction list（#287），Kotlin 七处 Use continuation indent（#288），Comment at first column、Indent 'case' branches 和 Kotlin context 参数续行（#289），Wrap first call 和 Group declarations 的字段 / 变量 / 赋值对齐（#290）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 86 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。Kotlin 格式化器在 plugins/kotlin/formatter/minimal/src/org/jetbrains/kotlin/idea/formatter/（KotlinCommonBlock.kt、kotlinSpacingRules.kt、KotlinLanguageCodeStyleSettingsProvider.kt），测试数据在 plugins/kotlin/idea/tests/testData/formatter/；容器里可以从 raw.githubusercontent.com 下载。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-10：`Wrapping::keep_multiple`（KEEP_MULTIPLE_EXPRESSIONS_IN_ONE_LINE，默认关，和 IntelliJ 一样）和 `modifier_wrap`（MODIFIER_LIST_WRAP，默认关），settings.conf wrapping 第 86–87 位（现在 88 位）。Multiple expressions：`split_one_line_bodies` 多了 `multiple` 参数，代码块里的行交给新的 `split_statements`，在括号外第一个后面还有代码的 `;` 处拆开（照 JavaSpacePropertyProcessor 两条语句之间 minLineFeeds = 1）。Wrap after modifier list：缩进之后的一遍 `wrap_modifier_lists`（在空行处理之后做，放在前面做会被最少空行的逻辑吃掉成员之间的空行），类体或顶层的成员行把开头的修饰符 / 注解拆到单独一行，拆了就再排一遍版（同 chop 之后的做法），让参数等对齐跟着走；缩进器里修饰符单独一行之后的 `Name(` 由 `modifier_list_line` / `constructor_paren` 认成构造函数声明，参数照样对齐到第一个下面。
- Signal 语料：默认 0 行变化（Java 5103、Kotlin 25404）；两项都打开时第二遍 0 行。
- 没做：`case 1: a(); break;` 这种 case 标签行、同一行 `{ a(); b(); }` 里的多条语句不拆（IntelliJ 会拆）；单独一行的带参数注解（`@Ann(x)`）后面的构造函数不当声明对齐；Align simple methods in columns、Builder methods、Text blocks 留给 sec33-reformat-rest-35。
