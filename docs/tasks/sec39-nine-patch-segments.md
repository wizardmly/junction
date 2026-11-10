---
id: sec39-nine-patch-segments
title: Drawable 预览：.9.png 按标记的可拉伸区域分段拉伸
features: 第 39 节
status: claimed
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec39-nine-patch-segments
pr:
claimed: 2026-11-10
files: src/domain/drawable.rs, src/domain/nine_patch.rs
---

## 要做什么

照 Android 的 `NinePatch`：读 `.9.png` 顶边和左边的黑色标记，得到横向、纵向的可拉伸区间；画的时候不可拉伸的部分保持原尺寸（按密度换成 dp），可拉伸的部分按各自的像素数分掉剩下的空间（目标比固定部分还小时按比例缩小固定部分）。现在是去掉标记边后整体拉伸。PNG 用已有依赖 flate2 解压，自己做行过滤还原，只支持 8 位非隔行（九宫格图通常如此），解不了时退回整体拉伸。

## 进度
