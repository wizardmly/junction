---
id: sec33-reformat-rest-21
title: Reformat：Wrapping and Braces 页剩下的选项（第二十一批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-21
pr: 278
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 for 语句的换行和对齐（FOR_STATEMENT_WRAP、ALIGN_MULTILINE_FOR、FOR_STATEMENT_LPAREN_ON_NEXT_LINE / RPAREN_ON_NEXT_LINE）、Special else if treatment（SPECIAL_ELSE_IF_TREATMENT）、Kotlin 的参数注解（PARAMETER_ANNOTATION_WRAP）。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276），Java Keep when reformatting 的 Simple blocks / methods / lambdas / classes 和 Control statement in one line（#277）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 36 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：Java 的 Special 'else if' treatment（`Wrapping::special_else_if`，settings.conf 第 36 位，默认开）、'for()' statement（`for_wrap: Wrap`，第 37 位）和 Align when multiline（`align_for`，第 38 位，默认开）。实现：`else_if_layout`（开：`else` / `} else` 单独一行加下一行 `if` 合并；关：拆开 `else if`，阶梯缩进交给缩进器，原来就支持不带花括号的语句体）；`wrap_for_statements`（for 头括号里顶层 `;` 后断，Chop 全断、Wrap if long 贪心，宽度按对齐后的列算）；缩进器的 `align` 除了 `declaration_paren` 还认 `for_paren`（`align_for` 开时）。设置页 Java 加一个下拉框、两个勾选框。
- 顺手修了旧 bug：`unary` 把 `step`、`to`、`in`、`as` 等 Kotlin 关键字也用在 Java 上，`i += step + step` 被格式化成 `step +step`；现在这些词只在 Kotlin 里算关键字。测试 `java_names_like_kotlin_keywords_take_binary_operators`。
- 回归：Signal 语料 Java 5107、Kotlin 24702 不变；Special else if 关 + for Chop / Wrap if long（对齐开 / 关）加实参 / 形参 Chop，第二遍都是 0 行。测试 `lays_out_else_if_and_for_statements`。
- 没做的（for 的 New line after '(' / Place ')' on new line、while / do-while / try-with-resources / assert 的换行、Kotlin 参数注解）挪到 sec33-reformat-rest-22。
