---
id: sec33-reformat-rest-22
title: Reformat：Wrapping and Braces 页剩下的选项（第二十二批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-22
pr: 279
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 for 的 New line after '(' / Place ')' on new line（FOR_STATEMENT_LPAREN_ON_NEXT_LINE / RPAREN_ON_NEXT_LINE）、try-with-resources（RESOURCE_LIST_WRAP）、assert（ASSERT_STATEMENT_WRAP）、Kotlin 的参数注解（PARAMETER_ANNOTATION_WRAP）。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274），Java Parameter annotations（#275），Kotlin Class / Method / Field / Local variable annotations（#276），Java Keep when reformatting 的 Simple blocks / methods / lambdas / classes 和 Control statement in one line（#277），Special 'else if' treatment、'for()' statement 换行和 Align when multiline（#278）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 39 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：Java 的 'try-with-resources'（`Wrapping::resource_wrap`，settings.conf 第 41 位）、Align when multiline（`align_resources`，第 42 位，默认开）、New line after '(' / Place ')' on new line（`resource_parens`，第 43–44 位）和 'for()' 的同名两项（`for_parens`，第 39–40 位）。`wrap_for_statements` 加了 keyword 参数，`try` 复用 `for` 的拆法；新函数 `header_parens` 对跨行的 `for (` / `try (` 头部在 `(` 后、`)` 前断行；`keyword_paren` / `keyword_open` 取代原来只认 for 的写法，`(` 后没有内容时不对齐（续行缩进）。缩进器的 `align` 多认 `try (`（`align_resources` 开时）。设置页 Java 加一个下拉框、五个勾选框。
- 回归：Signal 语料 Java 5107、Kotlin 24702 不变；for / try 都 Chop、括号两项全开或各开一项，加实参 / 形参 Chop，第二遍都是 0 行。测试 `wraps_resources_and_header_parentheses`。
- 没做的（assert 换行、Kotlin 参数注解、方法调用 / 声明括号换行）挪到 sec33-reformat-rest-23。
- 注意：容器磁盘配额满时 `rm -rf target/debug/incremental` 能腾出约 7 GB。
