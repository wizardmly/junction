---
id: sec41-merging-log
title: Merged Manifest 的 Merging Log：选中一行显示它的来源（ADDED / MERGED / INJECTED / IMPLIED from 哪个文件哪一行）
features: 第 41 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec41-merging-log
pr: https://github.com/wizardmly/junction/pull/262
claimed: 2026-11-09
files: src/domain/manifest_merge.rs, src/ui/file_editor/merged_manifest.rs
---

## 要做什么

照 Android Studio 的 Merged Manifest 视图：单击合并结果里的一行（元素或属性），右边栏出现 “Merging Log”，列出这个元素 / 属性是怎么来的：`ADDED from app/src/main/AndroidManifest.xml:5`、`MERGED from [lib] …`、`INJECTED from app/build.gradle.kts`、`IMPLIED from …`（老库隐含的权限），每条可点击打开那个文件那一行。

## 进度

- 2026-11-09：完成。
  - domain（`manifest_merge.rs`）：新增 `MergeAction`（Added / Merged / Injected / Implied，`label()` 给大写名）和 `LogEntry { action, source, line }`；内部的 `Element` / `Attr` 带 `log`，`MergedLine.log` 由 `print()` 填。
    - 解析时每个元素、属性记 ADDED（来源文件和行）；
    - `merge_into`（库合入）：相同的元素记 MERGED，值相同的属性记 MERGED；值不同（冲突或 `tools:replace`）不记；
    - `merge_higher`（overlay 合入）：元素记 MERGED；属性值相同记 MERGED，不同记 ADDED（overlay 的值赢）。`Element::set` 改成接收 `LogEntry`，顺带修了 overlay 覆盖的属性丢掉行号的问题；
    - build 文件注入的值和补出来的 `<uses-sdk>` 记 INJECTED；老库隐含权限记 IMPLIED。
    - 测试 `logs_where_each_line_came_from`。
  - UI（`merged_manifest.rs`）：选中的行存在 `MergedManifest.selected`（合并结果变了就清空，所以没动 file_editor.rs）；单击选中并高亮，右边栏在 Manifest Sources 下面显示 Merging Log，每条 “ACTION from 文件:行”，点击发 `FileEditorEvent::Navigate`；双击行打开来源的行为不变。
  - 实测：Linux（Xvfb）上 app + lib 两个模块，INTERNET 权限显示 ADDED from app…:3 / MERGED from lib…:3，allowBackup 显示 ADDED from app…:6 / MERGED from lib…:5，点 lib 那条打开 lib 的 manifest 第 5 行。
  - 没做：Android Studio 日志里的列范围（`5:5-10:15`）和 `[lib]` 这样的库名前缀。
