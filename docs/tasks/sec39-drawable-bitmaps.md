---
id: sec39-drawable-bitmaps
title: Drawable 预览：组合 drawable 里画位图（`<bitmap>`、`<nine-patch>`、PNG / JPEG / WebP 引用）
features: 第 39 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec39-drawable-bitmaps
pr: https://github.com/wizardmly/junction/pull/319
claimed: 2026-11-10
files: src/domain/drawable.rs, src/ui/file_editor/vector_preview.rs, src/ui/file_editor/previews.rs, src/usecase/android_resources.rs
---

## 要做什么

照 Android 的 `BitmapDrawable` / `NinePatchDrawable`：drawable 预览（Split / Design 视图和 gutter 缩略图）里，`<layer-list>`、`<selector>`、`<inset>` 等引用的位图（`@drawable/photo` 指向 png / jpg / webp / gif / .9.png）和 `<bitmap android:src>`、`<nine-patch android:src>` 都画出来：按所在密度目录换算成 dp，`<bitmap>` 的 `gravity`（默认 fill）、`tileMode`、`alpha`，`.9.png` 去掉 1 像素的标记边。

## 进度

- 2026-11-10：做完（PR #319）。`domain::drawable::Lookup` 加 `bitmap`（引用 → `Bitmap { bytes, density, nine_patch }`）；`Bitmap::density_of`（目录名里的 ldpi … xxxhdpi，tvdpi 1.33，其他为 1）、`image_size`（PNG IHDR、GIF、WebP 的 VP8 / VP8L / VP8X、JPEG 的 SOF 段，只读文件头，不解码）、自带的 base64（不加依赖）。`Writer::bitmap` 输出 `<image href="data:…">`：普通位图按 gravity 放在裁到所在矩形的 `<svg>` 里，tileMode 用 `<pattern>`，nine-patch 用 `viewBox="1 1 w-2 h-2"` 裁掉标记边后 `preserveAspectRatio="none"` 拉伸。`draw_reference` / `reference_intrinsic` 在找不到 drawable XML 时退到位图；`ROOTS` 加 `bitmap`、`nine-patch`。`usecase::android_resources::ResourceTable` 加 `bitmap`（同 `image`，但包括 `.9.png`）和 `load_bitmap`（读文件）；`vector_preview::resource_lookups` 多返回一个位图查找，gutter 缩略图（`previews.rs`）同样。测试 `draws_bitmaps`；Linux 实测见 FEATURES.md 第 39 节。
- 没做：九宫格按标记分段拉伸要解码像素（`image` 包没开 png 解码特性），现在整体拉伸；`tint` 等见 FEATURES.md 第 39 节的 ⬜ 行。
