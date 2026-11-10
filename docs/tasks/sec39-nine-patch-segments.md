---
id: sec39-nine-patch-segments
title: Drawable 预览：.9.png 按标记的可拉伸区域分段拉伸
features: 第 39 节
status: done
owner: Claude（项目线程）/ 替 Megan 干活
branch: task/sec39-nine-patch-segments
pr: https://github.com/wizardmly/junction/pull/324
claimed: 2026-11-10
files: src/domain/drawable.rs, src/domain/nine_patch.rs
---

## 要做什么

照 Android 的 `NinePatch`：读 `.9.png` 顶边和左边的黑色标记，得到横向、纵向的可拉伸区间；画的时候不可拉伸的部分保持原尺寸（按密度换成 dp），可拉伸的部分按各自的像素数分掉剩下的空间（目标比固定部分还小时按比例缩小固定部分）。现在是去掉标记边后整体拉伸。PNG 用已有依赖 flate2 解压，自己做行过滤还原，只支持 8 位非隔行（九宫格图通常如此），解不了时退回整体拉伸。

## 进度

- 2026-11-10：做完（PR #324）。新文件 `src/domain/nine_patch.rs`：`decode`（PNG 块解析、flate2 解压、五种行过滤还原，转 RGBA；只支持 8 位非隔行）、`regions`（顶边 / 左边不透明黑色像素的连续段，坐标去掉边框）、`pieces`（一个方向上固定段和拉伸段在目标里的位置）。`drawable::Writer::bitmap` 对能解码的九宫格在 `<defs>` 里放一次 `<image>`，再按行 × 列输出 `<svg viewBox=源区域 preserveAspectRatio="none"><use/></svg>`；解不了时仍是原来的整体拉伸。测试 `decodes_and_finds_regions`、`stretches_only_the_marked_parts`（测试里自己编码 PNG，覆盖五种过滤）、`draws_bitmaps` 里的九宫格部分；Linux 实测见 FEATURES.md 第 39 节。
- 没做：内容边距（padding，底边和右边的标记）；16 位和隔行 PNG。
