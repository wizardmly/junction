---
id: sec39-drawable-padding
title: Drawable 预览：.9.png 和 shape 的内容边距，layer-list 按边距嵌套
features: 第 39 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec39-drawable-padding
pr:
claimed: 2026-11-16
files: src/domain/drawable.rs, src/domain/nine_patch.rs
---

## 要做什么

FEATURES.md 第 39 节 ⬜ 行里的 “`.9.png` 的内容边距（底边、右边的标记）”。边距只在 layer-list 里看得出来：`LayerDrawable` 默认 `PADDING_MODE_NEST`，每一层的容器都要缩进前面各层的 `getPadding` 之和。

不做：`<bitmap>` 的 `mipMap` / `antialias`，以及库主题里的属性。

## 进度

- 2026-11-16：照 AOSP 源码核对规则：
  - `LayerDrawable.updateLayerBoundsInternal` 和 `getIntrinsicWidth`：nest 时累加前面各层的边距。
  - `RippleDrawable.inflate`：先 `setPaddingMode(PADDING_MODE_STACK)`。
  - `DrawableContainer.getPadding`：取 constant padding，也就是各项里最大的。
  - aapt `Images.cpp`：没有边距标记时，用第一段可拉伸区。
- 代码：
  - `nine_patch::Regions.padding`（像素）。
  - `drawable.rs` 的 `Writer::padding` / `reference_padding` / `item_padding`，以及 `stacked`、`bitmap_padding`。
  - layer-list 的 `draw` 和 `intrinsic` 按 nest 累加边距。
  - 坑：引用是颜色（`#f00`）时，不能去 bitmap lookup 里找边距。
- 测试：
  - `nests_layers_in_the_padding_of_earlier_ones`。
  - `decodes_and_finds_regions` 加了边距断言。
- Linux 实测见 FEATURES.md。
