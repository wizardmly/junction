# 任务板

一个任务一个文件，多个人或 agent 并行干活时用它来分工，不会互相撞车。规则见仓库根目录的 [AGENTS.md](../../AGENTS.md)。

- 看有哪些任务、谁在做：`python3 tools/tasks.py`（加 `--prs` 会同时查 GitHub 上打开的 PR）。
- 认领：从 master 拉分支 `task/<任务名>`，把任务文件改成 `status: claimed` 并填上 owner、branch、claimed，提交后马上推送这个分支，并开一个草稿 PR，标题以 `[<任务名>]` 开头。远程分支推送成功、草稿 PR 开出来，才算认领成功。
- 新任务：照下面的格式新建一个 `<任务名>.md`。任务名用小写字母、数字和 `-`，最好以 FEATURES.md 的节号开头（如 `sec35-…`）。新建任务可以单独开一个 PR，也可以随手放进别的 PR。
- 做完：最后一个 PR 里把 `status` 改成 `done`，填上 `pr`，进度一节写清做到哪、剩下什么。剩下的另建新任务文件。

```markdown
---
id: sec35-type-intentions        # 和文件名一致
title: 一句话说清要做什么
features: 第 35 节               # docs/FEATURES.md 里对应的节
status: open                     # open / claimed / done
owner:                           # 认领人（人名或 agent 名 + 替谁干活）
branch:                          # task/<id>
pr:                              # PR 号
claimed:                         # 认领日期 YYYY-MM-DD
files: src/domain/x.rs, src/ui/y.rs   # 预计会改的文件，用来避开别人
---

## 要做什么

## 进度
```
