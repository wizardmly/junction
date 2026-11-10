---
id: sec39-sweep-gradient
title: Drawable 预览：shape 的 sweep 渐变按锥形画
features: 第 39 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec39-sweep-gradient
pr:
claimed: 2026-11-10
files: src/domain/drawable.rs
---

## 要做什么

照 GradientDrawable 的 `SweepGradient`：`<gradient android:type="sweep">` 以 `centerX` / `centerY` 为中心，从 3 点钟方向顺时针转一圈，start / center / end 颜色均匀分布。SVG 没有锥形渐变，现在按 linear 画；改成一个 `<pattern>`，里面用很多细扇形各填插值后的颜色。

## 进度
