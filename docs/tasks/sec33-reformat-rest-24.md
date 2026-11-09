---
id: sec33-reformat-rest-24
title: Reformat：Wrapping and Braces 页剩下的选项（第二十四批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-24
pr: 281
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 Kotlin 的方法调用 / 声明的 New line after '(' / Place ')' on new line（Kotlin 官方风格默认开：声明的跟着 METHOD_PARAMETERS_WRAP，调用的用 dependent line feed 并排除 lambda 和 object，见 kotlinSpacingRules.kt）、Java 的 Parenthesized expression（PARENTHESES_EXPRESSION_LPAREN_WRAP 等）、Record components、Enum 的其他选项。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276），Java Keep when reformatting 的 Simple blocks / methods / lambdas / classes 和 Control statement in one line（#277），Special 'else if' treatment、'for()' statement 换行和 Align when multiline（#278），'try-with-resources' 和 for / try 的括号换行（#279），Java 'assert' statement、Java 方法调用 / 声明括号换行和 Kotlin Parameter annotations（#280）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 51 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。Kotlin 格式化器在 plugins/kotlin/formatter/minimal/src/org/jetbrains/kotlin/idea/formatter/（KotlinCommonBlock.kt、kotlinSpacingRules.kt、KotlinLanguageCodeStyleSettingsProvider.kt），测试数据在 plugins/kotlin/idea/tests/testData/formatter/；容器里可以从 raw.githubusercontent.com 下载。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：`list_parens` 加了 `kotlin` 和 `expr` 参数。Kotlin 调用照 kotlinSpacingRules.kt 的 dependent line feed：`(` 到 `)` 之间去掉 `{…}`（lambda、object）后还有换行就拆，单独一个 `object` 实参时 `(` 不拆；Kotlin 声明开着时保留换行，关掉时把 `(` 后、`)` 前的换行并回去（上一行有 `//` 注释时不并）。`chop` 的 Kotlin 分支按 `call_parens` / `param_parens` 决定第一项和 `)` 的位置。`Wrapping::KOTLIN` 的这两项按 IntelliJ 默认开，Signal 语料 0 行变化。Java 的 `expr_parens`（Parenthesized expression）：括号前不是名字、`)`、`]`、`>`，不是 if / for / while / switch / catch / synchronized / try 的条件，`)` 后不是 `->`。settings.conf 的 wrapping 现在 53 位（51–52 是 `expr_parens`；Kotlin 旧文件缺 47–50 位时按语言默认开）。`chop_long_lines` 加了保护：拆完还是一行的不再重试（Kotlin 关掉括号选项时单个实参会死循环）。
- 没做：Kotlin 关掉 `(` 选项时续行没有对齐到第一项下（用一级缩进）；Java 的 Record components、Deconstruction list，Kotlin 的 Use continuation indent，各种 Align when multiline，留给 sec33-reformat-rest-25。
