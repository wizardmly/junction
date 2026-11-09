---
id: sec41-merging-log
title: Merged Manifest 的 Merging Log：选中一行显示它的来源（ADDED / MERGED / INJECTED / IMPLIED from 哪个文件哪一行）
features: 第 41 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec41-merging-log
pr:
claimed: 2026-11-09
files: src/domain/manifest_merge.rs, src/ui/file_editor/merged_manifest.rs, src/ui/file_editor.rs
---

## 要做什么

照 Android Studio 的 Merged Manifest 视图：单击合并结果里的一行（元素或属性），右边栏出现 “Merging Log”，列出这个元素 / 属性是怎么来的：`ADDED from app/src/main/AndroidManifest.xml:5`、`MERGED from [lib] …`、`INJECTED from app/build.gradle.kts`、`IMPLIED from …`（老库隐含的权限），每条可点击打开那个文件那一行。

## 进度
