---
id: sec38-breakpoints-group-by-file
title: Breakpoints 对话框的 Group by File
features: 第 38 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec38-breakpoints-group-by-file
pr:
claimed: 2026-11-16
files: src/ui/breakpoints_dialog.rs, src/settings.rs, assets/as-icons/src/group-by-file*.svg, src/ui/as_icons_gen.rs
---

## 要做什么

FEATURES.md 第 38 节的 “异常断点按文件分组”。照 IntelliJ 的 `XBreakpointFileGroupingRule`（id `by-file`）：Breakpoints 对话框工具栏上的 Group by File 开关，打开后每个类型组里按文件再分一层，文件节点显示文件图标、文件名和所在目录。只有有源码位置的断点参与（IntelliJ 也是只对有 `getSourcePosition()` 的断点分组），所以异常断点和通配方法断点（`com.example.*Service.save()`）不分组。

## 进度

- 2026-11-16：
  - `breakpoints_dialog.rs`：`rows(entries, by_file)` 算出列表的行（`Row::Group` / `Row::File` / `Row::Entry`），`sort_key` 按类型组、文件名、完整路径、行号排序，同一个文件的断点排在一起（以前按标题排，两个同名文件的断点会交错）。
  - 工具栏 + / − 右边加 Group by File 开关（IntelliJ 的 `AllIcons.Actions.GroupByFile`，新图标 `GROUP_BY_FILE`），开关存在设置的 `breakpoints_group_by_file`，默认关（IntelliJ 默认也不选）。
  - 测试 `group_by_file_nests_each_files_breakpoints_inside_its_type_group`。
  - Linux 实测（Xvfb）：上面 FEATURES.md 那行写的场景，文件节点在类型组下，断点缩进在文件节点下，通配方法断点和异常断点不分组，开关状态写进 settings.conf。
