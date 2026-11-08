---
id: sec33-reformat-rest
title: Reformat：剩下的换行和代码风格页签
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest
pr: 236
claimed: 2026-11-08
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：值移到下一行后仍超长时的换行、Do not keep line breaks、Code Style 设置页剩下的页签（Spaces 的 Before left brace / Within / In ternary operator、Wrapping、Imports）。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归，改动行数不能变多。

## 进度

- 2026-11-08：这次只做 Java 的 Spaces › Before left brace（12 项）和 In ternary operator（4 项）。`domain::reformat::Spaces` 加 `before_left_brace: [bool; 12]`、`ternary: [bool; 4]`，`SPACES_LEN = 40`，`LABELS` / `JAVA` 顺序同 IntelliJ 设置页；`normalize` 里 `{` 按 `Spaces::before_brace` 判断是哪种结构（看匹配的 `(` 前的词、else / do / try / finally、throws、class 关键字），`?` / `:` 只在 Java 三元里（`ternary_question` 计数）按 `spaced_operator` 处理。设置页是通用的，按 `Spaces::JAVA` 和 `LABELS` 列出，不用改界面。Signal 60 个 Java 文件语料改动行数不变（21722）；Xvfb 实测见 FEATURES.md 第 33 节。
- 剩下的（值移过去仍超长的换行、Do not keep line breaks、Within、Kotlin 的 Before left brace、Wrapping、Imports）挪到新任务 sec33-reformat-rest-2。
