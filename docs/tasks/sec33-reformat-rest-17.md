---
id: sec33-reformat-rest-17
title: Reformat：Wrapping and Braces 页剩下的选项（第十七批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-17
pr: 274
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 if / for / while 语句体的换行（Place on new line / Special else if treatment）、参数注解（PARAMETER_ANNOTATION_WRAP）、Throws keyword（THROWS_KEYWORD_WRAP）、注解参数（Annotation parameters）、Kotlin 的注解换行。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 28 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：Java 的 Throws keyword（`Wrapping::throws_keyword`，settings.conf 第 28 位）和 Annotation parameters（`Wrapping::annotation_params`，第 29 位），设置页用 `wrap_option`。`chop` 里注解的括号（`annotation_paren`）改走 annotation_params，不再跟着 call_args；Throws keyword 用 `throws_at` 在 `throws` 前断行。缩进器：行首是 `throws ` / `implements ` / `extends ` 的 Java 行按 `Reason::Header` 续行缩进。
- 同一 PR 修了 `indent_unit`：先数行尾 `{` 的下一行多缩进 2 还是 4，样本 ≥ 3 时按多数定，不再管奇数列多不多（对齐的参数在奇数列，以前会把 2 空格文件当成乱的，给 4）。强制打开各种拆行时，第一遍拆完第二遍单位来回翻转的 4 个文件因此稳定了。Signal 语料：Java 改动行数 15768 → 5140，Kotlin 24934 → 24702，默认和强制选项第二遍都是 0 行。测试 `wraps_throws_keyword_and_annotation_parameters`、`keeps_two_spaces_with_aligned_parameters`。
- 没做的（if / for / while 语句体换行、参数注解、Kotlin 注解）挪到 sec33-reformat-rest-18。
