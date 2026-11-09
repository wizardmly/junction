---
id: sec49-multi-caret-shrink
title: 多光标下的 Shrink Selection：每个光标的选区各退回一步
features: 第 49 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec49-multi-caret-shrink
pr: 254
claimed: 2026-11-09
files: src/ui/file_editor/editing.rs, src/ui/file_editor.rs
---

## 要做什么

照 IntelliJ：多光标时 Extend Selection（Ctrl+W）每个光标各扩大一步，Shrink Selection（Ctrl+Shift+W）按步骤退回，每个光标各退回一步；扩大时合并成一个的选区，退回时恢复成原来的几个。

## 进度

- 2026-11-09：做完（PR #254）。`FileEditor::selection_steps` 改成每一步存所有光标的选区（排好序的 `Vec<Range>`）；`extend_selection` 单光标和多光标走同一条路：每个选区各扩大一步，设回后再取一次（合并后的）选区记为这一步；`shrink_selection` 比较当前所有选区和最后一步，相同就退回上一步（`restore_selection`：一个选区用 `set_selected_range`，多个用 `set_selection_ranges`）。Linux 实测两个光标扩大三次、退回两次都对。
