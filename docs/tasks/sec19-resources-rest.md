---
id: sec19-resources-rest
title: Android 资源：字符串折叠、框架资源 Find Usages、取色器选资源
features: 第 19 节
status: done
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec19-resources-rest
pr: 238
claimed: 2026-11-08
files: src/domain/android_resources.rs, src/ui/file_editor/previews.rs, src/ui/file_editor.rs
---

## 要做什么

docs/FEATURES.md 第 19 节的 ❌ 行：编辑器里把 R.string.x / @string/x 折叠显示成值（需要编辑器支持行内折叠，先评估 vendor 里的编辑器）、框架资源（android.R / @android:）的 Find Usages 和翻译、取色器的“选资源”页。三件可以分开开 PR。

## 进度

- 2026-11-08：这次只做了取色器的 Resources 页（三件里的一件）。`usecase::android_resources` 加 `ResourceTable::colors`、`ColorTarget` / `color_target`（一行里取色器改写的范围，以及能不能写字面量、能不能写资源、现在是不是引用）、`color_reference`；`ui/file_editor/previews.rs` 的 `ColorEdit` 改成自己的弹窗（Resources / Custom 页签，`open_color_edit`、`set_color_resource`、`render_color_edit`），Custom 页用 vendor 里新加的 `ColorPicker::render_panel`（gpui-component，标了 `Junction:`）。照 AOSP `AndroidAnnotatorUtil` 和 `ResourceChooserHelper.kt`。
- 剩下两件拆成新任务：sec19-string-folding（字符串折叠显示值）、sec19-framework-usages（框架资源的 Find Usages 和翻译）。
