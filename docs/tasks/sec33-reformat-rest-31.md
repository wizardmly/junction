---
id: sec33-reformat-rest-31
title: Reformat：Wrapping and Braces 页剩下的选项（第三十一批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-31
pr: 288
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 Kotlin 的 Use continuation indent（CONTINUATION_INDENT_IN_PARAMETER_LISTS 等，要先让 Kotlin 括号里的内容区分缩进和续行缩进）。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276），Java Keep when reformatting 的 Simple blocks / methods / lambdas / classes 和 Control statement in one line（#277），Special 'else if' treatment、'for()' statement 换行和 Align when multiline（#278），'try-with-resources' 和 for / try 的括号换行（#279），Java 'assert' statement、Java 方法调用 / 声明括号换行和 Kotlin Parameter annotations（#280），Kotlin 方法调用 / 声明括号换行和 Java Parenthesized expression（#281），Java 参数 / 实参的 Align when multiline 和 Record components（#282），Java Binary expressions 的 Align when multiline（#283），Ternary / Throws list / Extends list 的 Align when multiline（#284），Assignment / Chained method calls / Array initializer 的 Align when multiline（#285），Kotlin 'when' 分支对齐和多行分支后换行（#286），Java Deconstruction list（#287）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 72 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。Kotlin 格式化器在 plugins/kotlin/formatter/minimal/src/org/jetbrains/kotlin/idea/formatter/（KotlinCommonBlock.kt、kotlinSpacingRules.kt、KotlinLanguageCodeStyleSettingsProvider.kt），测试数据在 plugins/kotlin/idea/tests/testData/formatter/；容器里可以从 raw.githubusercontent.com 下载。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：Kotlin 的 `Wrapping::kotlin_cont: [bool; 7]`（settings.conf wrapping 第 72–78 位，缺省全关，照 IntelliJ 的 KotlinOfficialStyleGuide）。顺序：参数、实参、表达式体、链式调用、父类型列表、`if` 条件、Elvis。缩进器里 `kc(i)` 在开着时给续行缩进、关着时给普通缩进：`Reason::Chain` / `Elvis`（`?:` 开头的行）/ `Assign` / `Header` 的 size，`apply_brackets` 新参数 `kotlin_sizes`（`(` 前是 `if` 用条件的，`kotlin_declares` 认出 `fun name` / `class Name` / `constructor` 用参数的，别的用实参的）；`chop` 里 Kotlin 的实参 / 参数前缀、`elvis_break` / `supertype_break` / `chain_break` / `assignment_break` 的续行也按同样的开关。
- 复排：`reformat_pass` 的 `nested` 改成次数，切开长行后的复排如果又切开了行就再跑一遍，最多两遍。之前 Kotlin 续行缩进设成 8 时 Signal 语料第二遍有 60 行还会变（复排里新切开的行后面的内容没重新缩进）。
- 没做：CONTINUATION_INDENT_IN_CONTEXT_PARAMETER_LISTS（context 参数），留给 sec33-reformat-rest-32。
