---
id: sec39-bitmap-tint
title: Drawable 预览：<bitmap> / <nine-patch> 的 tint 与 tintMode
features: 第 39 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec39-bitmap-tint
pr:
claimed: 2026-11-10
files: src/domain/drawable.rs
---

## 要做什么

照 BitmapDrawable / NinePatchDrawable：`android:tint` 按 `android:tintMode`（默认 `src_in`，还有 `src_atop`、`src_over`、`multiply`、`screen`、`add`）给图片着色。用 SVG 滤镜（feFlood + feComposite / feBlend）实现。

## 进度
