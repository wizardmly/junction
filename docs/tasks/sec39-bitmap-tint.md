---
id: sec39-bitmap-tint
title: Drawable 预览：<bitmap> / <nine-patch> 的 tint 与 tintMode
features: 第 39 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec39-bitmap-tint
pr: https://github.com/wizardmly/junction/pull/PRNUM
claimed: 2026-11-10
files: src/domain/drawable.rs
---

## 要做什么

照 BitmapDrawable / NinePatchDrawable：`android:tint` 按 `android:tintMode`（默认 `src_in`，还有 `src_atop`、`src_over`、`multiply`、`screen`、`add`）给图片着色。用 SVG 滤镜（feFlood + feComposite / feBlend）实现。

## 进度

- 2026-11-10：做完。`drawable::Writer::tint_filter` 在 `<defs>` 里写一个覆盖所在区域的 `<filter>`（`color-interpolation-filters="sRGB"`，否则颜色会按线性空间偏掉）：`feFlood` 出 tint 色，再按模式 `feComposite`（in / atop / over，multiply 用 arithmetic k1=1，add 用 k2=k3=1）或 `feBlend screen` 和原图合成；`<bitmap>` / `<nine-patch>` 有 `android:tint` 时把图片包在 `<g filter>` 里。测试在 `draws_bitmaps`。
- 顺手修：layer-list 里有 `width` / `height` 但没写 `gravity` 的层，以前居中，Android 的 `LayerDrawable` 是靠左 / 靠上（只有写了 `center` 才居中）；`fill_horizontal` / `fill_vertical` 也按单个方向拉满。测试同在 `draws_bitmaps`。
- 没做：`<bitmap>` 的 `mipMap`、`antialias`（预览里看不出区别）。
