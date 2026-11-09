---
id: sec33-reformat-rest-6
title: Reformat：剩下的换行和代码风格页签（第六批）
features: 第 33 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec33-reformat-rest-6
pr: 253
claimed: 2026-11-09
files: src/domain/reformat.rs, src/domain/code_style.rs, src/ui/settings_dialog.rs
---

## 要做什么

docs/FEATURES.md 第 33 节的 ❌ 行：值移到下一行后仍超长时的换行、Do not keep line breaks、Wrapping 页、Imports 页。

Java Spaces 页的 Within 已在 sec33-reformat-rest-4（#249）和 sec33-reformat-rest-5（#251）做完。格式化改动用 HANDOFF.md 6.4 说的 Signal 语料回归：corpus 测试设 `JUNCTION_FMT_OUT` 把结果写到目录，用 diff 数改动行数，不能变多。

## 进度

- 2026-11-09：这次只做 Imports 页，其余（值移过去仍超长时的换行、Do not keep line breaks、Wrapping 页）拆到 sec33-reformat-rest-7。
  - `domain::imports::StarImports`（names / members / packages，`default_for` 是 IntelliJ 的 Java、Kotlin 默认值，`always` 处理 `pkg.*` 和含子包的 `pkg.**`）；`optimize_with(style, text, &stars)` 在去掉未用 import 后、排序前按 (是否 static, 所在包或类) 计数，达到数量或在包列表里的合成一个 `owner.*`；成员数用于 Java 静态 import 和 Kotlin 里倒数第二段大写（类成员）的 import；别名、行尾注释、已是 `*` 的不参与。`optimize` 只剩测试用（默认设置）。
  - `ImportsEdit` 加 `added`；`removed` 现在含被合并的条数。编辑器提示 “Removed N imports, added M imports”，Reformat 的提示也带上；光标行的计算按 removed − added。
  - `CodeStyle::star_imports / set_star_imports`，settings.conf 的 `code_style_java_imports` / `code_style_kotlin_imports`（与默认相同时不写）。
  - 设置页 Java / Kotlin 段下 “Imports:” 三行输入框，包列表用 `;` 分隔；搜索词加在 settings_dialog.rs 的 Code Style 列表。
  - 调用方：file_editor.rs 的 Optimize Imports、Reformat 里的 optimize、保存时自动优化，`usecase::clean_up_files::cleaned` 多一个 `&CodeStyle` 参数。
  - 坑：合并后两个 `*` 包里有同名类（`java.awt.List` / `java.util.List`）时 IntelliJ 会再加单独 import，这里不知道包里有哪些类，没做。
