# AGENTS.md

给在这个仓库里干活的 AI agent（Claude Code、Codex、Copilot、Cursor 等）和人看的规则。多人、多个 agent 可以同时开发，靠下面的任务板和认领锁避免互相冲突。

## 开始之前

1. 先读 [HANDOFF.md](HANDOFF.md)：现状、编译测试、代码结构、约定和坑。
2. 看任务板：`git fetch origin && python3 tools/tasks.py --prs`。任务在 [docs/tasks/](docs/tasks/)，一个任务一个文件，格式见 [docs/tasks/README.md](docs/tasks/README.md)。
3. 要做的功能在 [docs/FEATURES.md](docs/FEATURES.md) 里找到对应的节，看清现在是 ✅ / 🟡 / ❌ 和括号里的说明。
4. 行为拿不准时，去 IntelliJ IDEA / Android Studio 的开源代码里查它们怎么做（见 HANDOFF.md 第 8 节），不要凭印象。

## 认领：先拿锁再动手

只做自己认领到的任务，不碰别人认领的。

1. 选一个 `tools/tasks.py --open --prs` 里列出的任务（状态 open、远程没有 `task/<id>` 分支、没有 `[<id>]` 开头的 PR）。想做的事没有任务文件，就先新建一个。
2. 从最新 master 拉分支 `task/<id>`，把任务文件改成 `status: claimed`，填 `owner`（你的名字，agent 就写“agent 名 / 替谁干活”）、`branch`、`claimed`（日期），提交 `Claim <id>`。
3. **用只在分支不存在时才成功的方式推送**：

   ```sh
   git push --force-with-lease=task/<id>: -u origin task/<id>
   ```

   被拒绝（`stale info`）就说明别人刚认领了，换一个任务。不要加 `--force`，否则锁就失效了。没有本仓库推送权限（fork 开发）时，推到自己的 fork，然后马上开草稿 PR。
4. **马上开草稿 PR**，标题以 `[<id>]` 开头。远程分支和草稿 PR 就是锁，别人用 `tools/tasks.py --prs` 能看到。
5. 一个任务太大就拆：在任务文件里写清这次只做哪部分，剩下的新建任务文件。

锁的有效期：认领的分支 7 天没有新提交、PR 也没动静，别人可以在 PR 里留言问一句，3 天没人回就可以接手（把 owner 改成自己，在进度里写明接手）。

## 完成一件事时：把进度落到文档里

**每做完一件事（一个 PR、一个功能、一次修复），都要在同一个 PR 里把进度写进文档**，这样别的 agent 接手时不用翻聊天记录或猜。没写进度的工作视为没做完。

- **任务文件**（docs/tasks/<id>.md）的“进度”一节：按日期写做了什么、关键的类型 / 函数名、坑、剩下什么。整个任务做完时 `status: done`，填 `pr`；没做完的部分新建任务文件。
- **docs/FEATURES.md**：只改你这个任务对应的节。行改成 ✅ / 🟡，括号里写清做到哪、没做的部分和测过的平台（例如“Linux 实测，Windows 需真机验证”）。Diff / Merge 相关的同时更新 docs/DIFF_MERGE.md。
- **HANDOFF.md**：只在约定、构建方式、代码结构变了时改，日常进度不往里写（它是大家共用的文件，改多了就冲突）。

中途被打断或要交给别人时，也先把“做到哪、卡在哪、下一步是什么”写进任务文件、推到分支再停手。

## 避免冲突

- **一个任务一个分支一个 PR**，不要两个人推同一个分支。PR 小而快：做完一小步就开 PR、CI 绿就合入。
- **开 PR 前和合入前先合入最新 master**（`git fetch origin && git merge origin/master`），自己的分支怎么整理随意，但不要改写别人的分支。
- **公共文件只做小而独立的改动**，并且只改自己需要的那几行：
  - `docs/FEATURES.md`：只改自己任务对应的节；
  - `src/ui/file_editor.rs`、`src/ui/settings_dialog.rs`、`src/ui/workspace/search.rs`（Find Action 列表）、快捷键表：新加的动作、设置项尽量放在自己的模块里，这里只加注册的一两行；
  - `Cargo.toml` / `Cargo.lock`：不加新依赖，要加先问维护者。
- 冲突时：文档里两边追加的行都保留；代码冲突按 master 上的新写法改自己的部分，不要丢掉别人的改动。
- 发现要改的文件在别人认领的范围里（看任务文件的 `files`），先在对方的 PR 里说一声，或者等对方合入后再改。

## 其他约定

- 新功能按 HANDOFF.md 第 4 节的 Clean 架构分层（domain / usecase / infra / ui），`src/arch_tests.rs` 会检查依赖方向。
- 合入 master 会触发自动发版，所以合入前 `cargo build` 无新增警告、`cargo test` 全过、CI（linux + windows）是绿的。
- 文档用中文；代码注释和提交信息用英文，风格照 `git log`。
