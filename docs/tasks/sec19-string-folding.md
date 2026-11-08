---
id: sec19-string-folding
title: Android 资源：编辑器里把字符串引用折叠显示成值
features: 第 19 节
status: open
owner:
branch:
pr:
claimed:
files: src/ui/file_editor.rs, vendor/gpui-component（编辑器折叠）
---

## 要做什么

docs/FEATURES.md 第 19 节的 ❌ 行：像 AS 一样把代码里的 `R.string.x` / `getString(R.string.x)` 和 XML 里的 `@string/x` 折叠显示成字符串的值，光标进去时展开。需要编辑器支持行内折叠（现在只有整行折叠），先评估 vendor 里的编辑器怎么加。行为照 AOSP 的 `ResourceFoldingBuilder`。

## 进度

（认领后在这里按日期记：做了什么、卡在哪、下一步。）
