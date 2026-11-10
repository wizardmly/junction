---
id: sec32-vector-sweep-gradient
title: Vector drawable 预览：aapt 渐变的 sweep 类型按锥形画
features: 第 32 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec32-vector-sweep-gradient
pr: https://github.com/wizardmly/junction/pull/329
claimed: 2026-11-10
files: src/domain/vector_drawable.rs, src/domain/drawable.rs
---

## 要做什么

`<vector>` 里 `<aapt:attr name="android:fillColor">` 的 `<gradient android:type="sweep">` 现在只用第一个颜色填充。改成和 shape 的 sweep 渐变（sec39-sweep-gradient）一样画锥形：以 `centerX` / `centerY`（视口坐标）为中心，从 3 点钟方向顺时针，按 `<item>` 的 offset 或 start / center / end 插值。

## 进度

- 2026-11-10：做完。扇形图案从 `drawable.rs` 挪到 `vector_drawable::sweep_pattern`（连同 `interpolate`），shape 和 vector 共用；半径按中心偏离区域中心的距离放大，偏心时也盖满。vector 的 `Writer` 记下视口大小，图案铺满视口（路径坐标就在视口里）。测试在 `converts_to_svg`。Linux 实测：96 dp 圆形红→黄→青→红的 sweep 平滑无缝。
- 没做：`<group>` 带变换时图案仍按路径自己的坐标铺满视口，变换后超出视口的部分会重复图案（少见）。
