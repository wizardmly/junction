---
id: sec33-reformat-rest-18
title: Reformat：Wrapping and Braces 页剩下的选项（第十八批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-18
pr: 275
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：IntelliJ Code Style › Wrapping and Braces 页剩下的选项，例如 if / for / while 语句体的换行（Place on new line / Special else if treatment）、参数注解（PARAMETER_ANNOTATION_WRAP）、Kotlin 的注解换行。挑常用的先做，一批做几项。

已有的：实参、形参、链式调用、父类列表、赋值（#256），Keep line breaks（#259），大括号位置和 else / while / catch / finally on new line（#261），Force braces 和 Kotlin Put left brace on new line（#263），Java Binary expressions / Ternary operation（#266），Java Class / Method / Field annotations（#268），Java Enum constants（#269），Kotlin Elvis expressions（#270），Java Local variable annotations 和 Array initializer（#272），Java Throws list 和注解里的数组（#273），Java Throws keyword 和 Annotation parameters（#274）。类型是 `domain::reformat::Wrapping` / `Braces` / `Force` / `Operations` / `AnnotationWrap`，settings.conf 的 `code_style_<lang>_wrapping` 往后追加位数（现在 30 位，旧的照样读，缺的位用语言默认值）。设置页单个 Wrap 下拉框用 `wrap_option`。

IntelliJ 源码：容器里 /home/claude/jetbrains/intellij-community 是稀疏检出，Java 格式化器不在工作区，用 `git show HEAD:java/java-frontback-impl/src/com/intellij/psi/formatter/java/<文件>` 读（例如 AbstractJavaBlock.java、JavaFormatterUtil.java）。

格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多（变多要能说明是照 IntelliJ 改对了）。这个测试也跑第二遍，有变化就失败；新选项打开时（临时改 `Wrapping::JAVA` / `KOTLIN` 默认值跑）也要 0 行。

## 进度

- 2026-11-09：Java 的 Parameter annotations（`Wrapping::param_annotations: AnnotationWrap`，settings.conf 第 30 位），设置页放在 `annotation_options` 第 6 项。实现是 `wrap_parameter_annotations`：从 `declaration_paren` 认出的方法头的 `(` 起跨行跟踪参数，参数开头的 `leading_annotations` 每个后面是断点；Wrap always 全断，Chop down if long 行超长时全断，Wrap if long 按右边界贪心断；断出的行先对齐到第一个参数（宽度按对齐后的列算，否则第二遍会再断），之后缩进器照常对齐。
- `indent_unit` 再改：`{` 后多缩进 2 和 4 的次数只要不相等就按多数定（以前要 ≥ 3 个样本），只有一个类的小接口文件里对齐到奇数列的参数不再把单位翻成 4。
- 回归：Signal 语料 Java 改动行数 5140 → 5107，Kotlin 24702 不变；默认、和实参 / 形参 Chop + 链式 Wrap if long 一起把 Parameter annotations 设成三种模式，第二遍都是 0 行。测试 `wraps_parameter_annotations`。
- 没做：`catch (@A E e)`、lambda 参数、无修饰符的包内构造函数 `A(…)`（`declaration_paren` 认不出）；if / for / while 语句体换行、Kotlin 注解挪到 sec33-reformat-rest-19。
