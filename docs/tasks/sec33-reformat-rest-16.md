---
id: sec33-reformat-rest-16
title: Reformat：Wrapping and Braces 页剩下的选项（第十六批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-16
pr: 273
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 if / for / while 语句体的换行（Place on new line / Special else if treatment）、参数注解（PARAMETER_ANNOTATION_WRAP）、Throws list、注解里的数组（`@Ann({a, b})`，IntelliJ 也走 ARRAY_INITIALIZER_WRAP）、Kotlin 的注解换行。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 27 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java）。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：这批做了 Java 的 Throws list 和注解里的数组，其余拆到 sec33-reformat-rest-17。
  - `Wrapping.throws_list: Wrap`（IntelliJ 的 THROWS_LIST_WRAP，默认 Do not wrap），settings.conf 第 28 位，设置页 `wrap_option` 一个下拉框。`throws_break`：方法头 `) throws A, B, C` 超过右边界时在逗号后断开，第一个类型留在 `throws` 后，其余续行缩进；在 `chop` 里实参 / 形参列表都不用拆时才用，`{` / `;` 跟在最后一个后面。
  - 缩进器：上一行是 `… ) throws A,` 或 Java 类头 `… implements A,` / `extends A,` 时，这行算 `Reason::Header`（`java_header_list`），按续行缩进，后面的方法体 / 类体仍按声明缩进。原来手写成多行的 throws / implements 会被缩成和声明同级。
  - 注解里的数组（`@SuppressWarnings({…})`、`@Config(sdk = {…})`）：`is_array_initializer` 认出注解括号里的 `{`，和数组一样走 Array initializer（IntelliJ 的 ANNOTATION_ARRAY_INITIALIZER 也用 ARRAY_INITIALIZER_WRAP），换行的元素按续行缩进。只认 `@A({` 和 `@A(x = {`；`@A(a = 1, b = {` 这种第二个参数还不认。
  - 回归：Signal Java 15768、Kotlin 24934 不变；Throws list、Array initializer 都设 Chop down if long 时第二遍 0 行。
  - Xvfb：settings.conf 写入两项后 Reformat Code 拆开 throws 和注解数组，再按一次提示没有改动。
