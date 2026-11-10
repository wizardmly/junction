---
id: sec39-sweep-gradient
title: Drawable 预览：shape 的 sweep 渐变按锥形画
features: 第 39 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec39-sweep-gradient
pr: https://github.com/wizardmly/junction/pull/PRNUM
claimed: 2026-11-10
files: src/domain/drawable.rs
---

## 要做什么

照 GradientDrawable 的 `SweepGradient`：`<gradient android:type="sweep">` 以 `centerX` / `centerY` 为中心，从 3 点钟方向顺时针转一圈，start / center / end 颜色均匀分布。SVG 没有锥形渐变，现在按 linear 画；改成一个 `<pattern>`，里面用很多细扇形各填插值后的颜色。

## 进度

- 2026-11-10：做完。`drawable::Writer::gradient` 的 `sweep` 分支写一个覆盖形状的 `<pattern>`（`patternUnits="userSpaceOnUse"`，内容平移 `-x -y`，因为图案内容的原点在图块左上角），里面 180 个从中心出发、半径为对角线两倍的三角扇形，每个比 2° 多 1°，避免抗锯齿缝；颜色由新函数 `interpolate` 按 `(offset, rgba)` 色标逐通道线性插值。三个颜色时中间色在 0.5（Android 的 SweepGradient positions 为 null，均匀分布）。测试在 `draws_shapes`。Linux 实测：80 dp 圆形红→绿→蓝从 3 点钟顺时针过渡平滑、没有缝；`centerX=0.25` 的圆角矩形中心偏左。
- 没做：`<vector>` 里 `<aapt:attr>` 的 sweep 渐变仍用首色（第 38 节，那边的渐变走 vector_drawable 的另一套代码）；`useLevel`。
