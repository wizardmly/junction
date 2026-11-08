# AGENTS.md

给在这个仓库里干活的 AI agent（Claude、Codex、Copilot 等）看的规则。人也适用。

## 开始之前

1. 先读 [HANDOFF.md](HANDOFF.md)：现状、编译测试、代码结构、约定和坑，特别是第 6.4 节“交接时的进度”。
2. 要做的功能先在 [docs/FEATURES.md](docs/FEATURES.md) 里找到对应的行，看清现在是 ✅ / 🟡 / ❌ 和括号里的说明。
3. 行为拿不准时，去 IntelliJ IDEA / Android Studio 的开源代码里查它们怎么做（见 HANDOFF.md 第 8 节），不要凭印象。

## 完成一件事时：把进度落到文档里

**每做完一件事（一个 PR、一个功能、一次修复），都要在同一个提交 / PR 里把进度写进进度文档**，这样别的 agent 接手时不用翻聊天记录或猜。没写进度的工作视为没做完。

要更新的地方：

- **docs/FEATURES.md**：对应行改成 ✅ / 🟡，括号里写清做到哪、没做的部分和测过的平台（例如“Linux 实测，Windows 需真机验证”）。新功能加新行；剩下没做的拆成单独的 ❌ 行。Diff / Merge 相关的同时更新 docs/DIFF_MERGE.md。
- **HANDOFF.md 第 6.4 节**：
  - 在进度段落末尾追加一句 `#PR号 做了什么`，写上接手的人需要知道的关键类型 / 函数名和坑；
  - 同步改“下一步照顺序接着做”的列表：做完的划掉或删掉，新发现的缺口加进去。
- **HANDOFF.md 开头的“最后更新”**：改成当天日期和这次做的事。
- **HANDOFF.md 第 7 节时间线**：只有阶段性的里程碑才加一行。

中途被打断或要交给别人时，也先把“做到哪、卡在哪、下一步是什么”写进第 6.4 节再停手。

## 其他约定

- 新功能按 HANDOFF.md 第 4 节的 Clean 架构分层（domain / usecase / infra / ui），`src/arch_tests.rs` 会检查依赖方向。
- 从 master 拉分支开发，PR 合入 master；合入会触发自动发版，所以合入前 `cargo build` 无新增警告、`cargo test` 全过、CI（linux + windows）是绿的。
- 文档用中文；代码注释和提交信息用英文，风格照 `git log`。

## 多个 agent 并行

1. **先认领再动手**：开工前在 HANDOFF.md 第 6.4 节的“正在进行”表里加一行（谁 / 分支 / FEATURES.md 第几节 / 开始日期），单独提交并合入 master（或推到自己的分支后马上开 PR），别人开工前先看这张表，挑没人认领的节。做完或放弃时在同一个 PR 里删掉这一行。
2. **一个 agent 一个分支**：分支名带上主题（如 `agent/sec35-intentions`），不要两个 agent 推同一个分支。
3. **不碰同一批文件**：认领按 FEATURES.md 的节划分；要改别人认领范围里的文件，先在 PR 描述里说明，尽量只做小改动。
4. **PR 小而快**：做完一小项就开 PR、CI 绿就合入，开 PR 前先合入最新 master；进度文档冲突时保留双方的行（都是追加的内容）。
