---
id: sec39-drawable-bitmaps
title: Drawable 预览：组合 drawable 里画位图（`<bitmap>`、`<nine-patch>`、PNG / JPEG / WebP 引用）
features: 第 39 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec39-drawable-bitmaps
pr:
claimed: 2026-11-10
files: src/domain/drawable.rs, src/ui/file_editor/vector_preview.rs, src/ui/file_editor/previews.rs, src/usecase/android_resources.rs
---

## 要做什么

照 Android 的 `BitmapDrawable` / `NinePatchDrawable`：drawable 预览（Split / Design 视图和 gutter 缩略图）里，`<layer-list>`、`<selector>`、`<inset>` 等引用的位图（`@drawable/photo` 指向 png / jpg / webp / gif / .9.png）和 `<bitmap android:src>`、`<nine-patch android:src>` 都画出来：按所在密度目录换算成 dp，`<bitmap>` 的 `gravity`（默认 fill）、`tileMode`、`alpha`，`.9.png` 去掉 1 像素的标记边。

## 进度
