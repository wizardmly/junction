---
id: sec33-reformat-rest-7
title: Reformat：剩下的换行和 Wrapping 页签（第七批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-7
pr: 256
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：值移到下一行后仍超长时的换行、Do not keep line breaks、Wrapping 页（IntelliJ Code Style 的 Wrapping and Braces）。

Imports 页已在 sec33-reformat-rest-6（#253）做完。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-09：这次只做 Wrapping and Braces 里和换长行有关的五项，其余（值移过去仍超长、Do not keep line breaks、页上其他选项）拆到 sec33-reformat-rest-8。
  - `domain::reformat::{Wrap, Wrapping}`（`Wrapping::JAVA` / `KOTLIN` 是 IntelliJ 默认），`IndentStyle.wrapping`（`None` 时按语言默认）；`chop_long_lines` 现在对 Java 也跑（任一项不是 Do not wrap 时），参数是 `Wrapper`（语言、缩进单位、Java 续行缩进、右边界、选项）。
  - `chop`：按每个 `(` 是声明形参还是调用实参取对应选项；Java 只有一个参数的列表不拆；拆法见 FEATURES.md。列表拆分抽成 `list_items`。拆不了时依次试 supertype_break（只 Kotlin）、chain_break、assignment_break，各自受选项控制；Java 用续行缩进。
  - `chain_break` 新规则：同一个表达式里前面已经有调用（`a.b().c()`、`f(x).g()`）才在点前换行，空格和运算符开始新表达式；这改变了 Kotlin 默认结果（更接近 IntelliJ）。
  - `CodeStyle::wrapping / set_wrapping`，settings.conf 的 `code_style_{java,kotlin}_wrapping`；设置页每项一个下拉菜单（PopupMenuItem）。
  - 回归：Signal 400 个 Java 文件默认设置下结果不变；临时把 Java 选项全打开跑语料，二次格式化不变。Kotlin 200 个文件见 FEATURES.md。
