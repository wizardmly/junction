---
id: sec19-string-folding
title: Android 资源：编辑器里把字符串引用折叠显示成值
features: 第 19 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec19-string-folding
pr: 241
claimed: 2026-11-08
files: src/ui/file_editor.rs, vendor/gpui-component（编辑器折叠）
---

## 要做什么

docs/FEATURES.md 第 19 节的 ❌ 行：像 AS 一样把代码里的 `R.string.x` / `getString(R.string.x)` 和 XML 里的 `@string/x` 折叠显示成字符串的值，光标进去时展开。需要编辑器支持行内折叠（现在只有整行折叠），先评估 vendor 里的编辑器怎么加。行为照 AOSP 的 `ResourceFoldingBuilder`。

## 进度

- 2026-11-08 完成（PR #241）：
  - 编辑器行内折叠：vendor/gpui-base 里本来就有给单行输入框用的 inline token（一段文字显示成一个标签，见 `inline_tokens.rs` / `element.rs` 的 `measure_tokens`、`layout_token_row`），代码编辑器不支持。加了 `InputBaseState::set_display_folds(Vec<(Range, label)>)`：`InlineTokenStore.display = true` 的 token 只是显示，不进文档：编辑范围不会被扩成整个 token（`normalize_token_range`），撤销历史不记（`edit_tokens` 返回 None、`replay_tokens` 跳过、undo 不再关事件），碰到它的编辑把它删掉；`layout_lines` 只对有折叠的行走 token 排版，别的行照旧（保留背景色等）；点折叠块把光标放到开头而不是选中整个 token；折叠块画成浅灰底圆角框。都标了 `Junction:`。
  - 找引用和算值：`src/usecase/resource_folding.rs` 的 `folds(table, path, text)`，规则照 AOSP（Kotlin 版 `ResourceFoldingBuilder.kt`、Java 版 `ResourceFoldingBuilder.java` + `InlinedResource.java`）：调用名单、接收者链、`insertArguments` 的 `{参数}`、60 字符截断、`名字: 值`、XML 里带引号整段折叠、bool 只在 XML。
  - 接进编辑器：`src/ui/file_editor/resource_folds.rs`，在 `update_previews` 的后台任务里一起算（用同一个 ResourceTable），按编辑计数 `server_edits` 判断是否过期；`follow_resource_folds` 在光标 / 选区所在行变化时重新下发（那几行不折叠）。设置项 `DiffSettings.fold_android_strings`（键 `diff_fold_android_strings`，默认开），Settings 新页 Editor › Code Folding。
  - 坑：undo 时 `push_history` 已经调了 `edit_tokens` 移动折叠，`replay_tokens` 再移一次会错位（折叠跨行导致排版越界崩溃），所以 display 折叠在 `replay_tokens` 里直接返回；排版里也过滤掉跨行的 token 防崩。
  - 测试：单测 `folds_kotlin_and_java_references`、`folds_xml_attributes_and_shortens`；Xvfb 打开 Kotlin 和 layout XML 看折叠、光标进出展开、行尾输入 + 回车 + 三次撤销、点击折叠块都正常。
  - 没做：plurals（资源扫描不存 plurals 的值）、对单个字符串折叠用 Ctrl+- / Ctrl+= 收起展开。
