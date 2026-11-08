---
id: sec19-resources-rest
title: Android 资源：字符串折叠、框架资源 Find Usages、取色器选资源
features: 第 19 节
status: claimed
owner: Claude（项目线程）/ 替 Jake 干活
branch: task/sec19-resources-rest
pr:
claimed: 2026-11-08
files: src/domain/android_resources.rs, src/ui/file_editor/previews.rs, src/ui/file_editor.rs
---

## 要做什么

docs/FEATURES.md 第 19 节的 ❌ 行：编辑器里把 R.string.x / @string/x 折叠显示成值（需要编辑器支持行内折叠，先评估 vendor 里的编辑器）、框架资源（android.R / @android:）的 Find Usages 和翻译、取色器的“选资源”页。三件可以分开开 PR。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
