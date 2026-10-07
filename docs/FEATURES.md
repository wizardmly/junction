# GitGlass 功能清单：一比一复刻 Android Studio / IntelliJ 的 Git 客户端

对照基准：Android Studio（基于 IntelliJ Platform，新 UI）里的 Git 功能，也就是 git4idea + VCS Log + Commit 工具窗口 + Diff/Merge 工具。
状态：✅ 已实现　🟡 部分实现　⬜ 未开始。阶段：M1–M6，见文末。

## 1. 窗口外壳（新 UI）

| 功能 | 细节 | 阶段 | 状态 |
|---|---|---|---|
| 玻璃窗口 | macOS vibrancy / Windows Mica-Acrylic / Linux 半透明（KDE 模糊） | M1 | 🟡 |
| 自定义标题栏 | 主菜单（汉堡按钮）、项目名组件、VCS 分支组件、右侧工具按钮 | M1 | 🟡 |
| 项目组件 | 最近项目列表、打开、克隆、新建仓库 | M1/M4 | ✅（欢迎页 + 项目组件下拉：最近项目、Open、Get from VCS、Create Git Repository） |
| VCS 分支组件 | 当前分支名、进行中的操作（Rebasing/Merging/Cherry-picking）、点击弹出分支弹窗 | M1 | 🟡 |
| 左/右/下 工具窗口条 | Commit、Git（Log/Console）、可拖拽、可隐藏、记住尺寸 | M1 | 🟡 |
| 状态栏 | 分支、行分隔符、后台任务进度、通知 | M1 | 🟡 |
| VCS 操作弹窗 | `Alt+\``（macOS `Ctrl+V`）快速操作列表 | M3 | ✅（数字键快选未做） |
| 通知气泡 | 操作结果、错误、可点击的动作（View、Undo、Show details） | M2 | ✅（View Commit、Show Details、Update Project、提交后 Undo） |
| 主题 | 亮/暗、跟随系统、Int UI 配色、紧凑模式 | M1 | 🟡 |
| 快捷键 | 与 IntelliJ 默认 keymap 一致（`Ctrl+K` 提交、`Ctrl+Shift+K` 推送、`Ctrl+T` 更新…） | M2 | 🟡（Ctrl+K、Ctrl+Shift+K、Ctrl+T、Ctrl+Shift+\`、Alt+9、F7/Shift+F7） |

## 2. Git 工具窗口 › Log

### 2.1 分支面板（左侧，可隐藏）
| 功能 | 阶段 | 状态 |
|---|---|---|
| 树：HEAD (Current Branch)、Local、Remote（按 remote 分组）、Tags | M1 | ✅ |
| 收藏分支（星标）、置顶 | M2 | ✅（分支弹窗与 Log 分支面板：星标切换，收藏排在组内最前；默认收藏 main/master） |
| 按 `/` 分组为目录（feature/xxx） | M1 | ✅ |
| 分支搜索（直接输入） | M1 | ✅（Log 分支面板顶部搜索框，过滤并展开所有匹配；分支弹窗打开即聚焦搜索） |
| 单击定位到分支顶端、双击按分支过滤 Log | M1 | ✅ |
| ahead/behind 指示（↑↓ 箭头） | M2 | ✅ |
| 工具栏：New Branch、Update Selected、Delete、Compare with Current、Show My Branches、Fetch、展开/折叠 | M2 | ✅（New Branch、Fetch、Update Selected、Delete、Compare with Current、Show My Branches、展开/折叠、按分支过滤） |
| 多仓库根时按仓库分组 | M5 | ⬜ |
| 右键菜单：与分支弹窗动作一致（见 §4） | M2 | ✅（与分支弹窗相同的动作 + Add to / Remove from Favorites） |

### 2.2 提交列表（中间）
| 功能 | 阶段 | 状态 |
|---|---|---|
| 提交图（彩色泳道、合并线、长边折叠为箭头） | M1 | ✅ |
| 列：Subject（含分支/标签标签）、Author、Date、Hash；列可显示/隐藏、可拖宽 | M1 | 🟡（View Options › Show Columns：Author / Date / Hash 显示隐藏并记住；拖宽未做） |
| 引用标签：本地分支、远程分支、标签、HEAD；左/右侧显示；紧凑引用视图 | M1 | ✅（本地/远程/标签/HEAD 标签；View Options：Compact References View、Show References on the Left） |
| 虚拟滚动，几十万提交流畅 | M1 | ✅ |
| 分段加载（先加载最近的，滚动时加载更多） | M2 | ✅（先 1000 条，其余后台加载） |
| 多选（Shift/Ctrl） | M2 | ✅（多选后可 Cherry-Pick / Revert / 复制哈希） |
| 高亮：我的提交（粗体）、合并提交（灰色）、当前分支提交、未合并到当前分支的提交 | M2 | ✅（View Options › Highlight：My Commits（粗体）、Merge Commits（灰色）、Current Branch（底色）、Not Merged into Current Branch（灰色）） |
| IntelliSort / 按拓扑 / 按日期排序 | M2 | ✅（View Options › Sort：IntelliSort（拓扑序）/ By Date（--date-order）） |
| 折叠/展开线性分支、显示长边 | M3 | ✅（View Options → Collapse Linear Branches，“⋯ N commits”点击展开） |
| 日期格式：相对时间 / 绝对时间 | M1 | ✅（View Options › Relative Dates：“5 minutes ago”；否则 Today/Yesterday/日期） |
| `Ctrl+F` 跳转到 hash / 分支 / 标签 | M2 | ✅ |
| 多个 Log 标签页（从分支打开新标签） | M3 | ✅ |

### 2.3 过滤栏
| 功能 | 阶段 | 状态 |
|---|---|---|
| 文本/哈希搜索，选项：正则、区分大小写 | M1 | ✅（搜索框内 Cc（区分大小写）、.*（正则）开关；hash 前缀也能搜） |
| Branch 过滤（多选、收藏） | M1 | ✅（All、HEAD、Favorites、Select…（多选对话框）、Recent、本地/远程列表） |
| User 过滤（me、作者列表） | M2 | ✅ |
| Date 过滤（最近 24h/7 天/自定义） | M2 | ✅（最近 24h / 7 天 / 30 天 / 1 年；Select… 自定义 From / To） |
| Paths 过滤（结构过滤：选择目录/文件） | M2 | ✅（Paths 下拉：All、Select Folders…（文件或目录，可多选）；文件夹不加 --follow） |
| 过滤历史记录 | M3 | ✅（Branch / User / Paths 下拉里的 Recent：最近 5 个过滤条件（当前会话）） |

### 2.4 提交详情 + 变更树（右侧）
| 功能 | 阶段 | 状态 |
|---|---|---|
| 变更文件树：按目录分组、按模块分组、扁平列表；文件状态颜色（新增绿、修改蓝、删除灰、重命名） | M1 | 🟡 |
| 详情：完整提交信息、hash、作者/提交者、日期、包含该提交的分支、标签 | M1 | ✅（作者与提交者不同时显示提交者） |
| 双击文件打开 Diff；Diff 预览（编辑器区或面板内） | M2 | ✅ |
| 多选提交时显示合并后的变更 | M3 | ✅ |
| 签名信息（GPG 验证） | M4 | ✅（详情面板显示 Verified / Bad / 无法校验 与签名者、key；GPG 与 SSH 签名） |
| 提交信息里的 issue 链接 / URL 可点击 | M3 | ✅（URL 和提交哈希） |

### 2.5 提交右键菜单
| 动作 | 阶段 | 状态 |
|---|---|---|
| Copy Revision Number（含 `Ctrl+C`） | M1 | ✅ |
| Create Patch… | M4 | ✅（多选时合并为一个补丁；保存到文件或剪贴板，可反向） |
| Cherry-Pick | M3 | ✅（冲突走进行中操作横幅） |
| Checkout Revision | M2 | ✅ |
| Show Repository at Revision | M4 | ✅（提交右键 → 显示该版本的文件树，双击打开只读编辑器） |
| Compare with Local | M3 | ✅ |
| Reset Current Branch to Here…（Soft / Mixed / Hard / Keep） | M2 | ✅ |
| Revert Commit | M3 | ✅（冲突走进行中操作横幅） |
| Undo Commit（最新的未推送提交） | M2 | ✅ |
| Edit Commit Message…（reword） | M3 | ✅ |
| Fixup… / Squash Into… | M3 | ✅（预填 fixup!/squash! 提交信息，交互式 Rebase 自动归位） |
| Drop Commits | M3 | ✅ |
| Squash Commits…（多选） | M3 | ✅ |
| Interactively Rebase from Here… | M3 | ✅ |
| Push All up to Here… | M3 | ✅ |
| New Branch… / New Tag… | M2 | ✅ |
| Go to Child Commit / Go to Parent Commit | M2 | ✅（提交右键 Go to Child Commit / Go to Parent Commit） |
| Open on GitHub/GitLab | M5 | ⬜ |

## 3. Commit 工具窗口（非模态提交）

| 功能 | 阶段 | 状态 |
|---|---|---|
| 变更树：Changes（changelist）、Unversioned Files、Ignored Files | M1 | ✅（Changes、Unversioned Files、Ignored Files 可在 View Options 显示） |
| 复选框选择要提交的文件；全选 | M1 | 🟡 |
| 分组：目录 / 模块 / 仓库；展开全部/折叠全部 | M2 | 🟡（按目录 / 扁平；展开全部、折叠全部；模块、仓库分组随 M5） |
| 多个 Changelist：新建、移动文件到、设为活动 | M4 | ✅（New / Edit / Delete / Set Active；Move to Another Changelist；活动列表粗体，新改动进入活动列表） |
| Staging 模式（启用暂存区）：Staged / Unstaged 两棵树，Stage/Unstage 按钮 | M2 | ✅ |
| 部分提交：按 chunk / 按行勾选（diff 中的复选框） | M4 | ✅（changelist 模式：diff 中每个 chunk 有复选框，提交时用临时 index 只提交勾选的 chunk；暂存区模式按 chunk Stage / Unstage） |
| 提交信息编辑器：拼写检查、右边距线、首行长度提示、提交信息历史（`Ctrl+M`） | M1/M3 | 🟡（等宽字体、右边距线、首行长度提示、Ctrl+M 历史；缺拼写检查） |
| Amend 复选框（自动载入上次提交信息） | M1 | ✅ |
| Commit / Commit and Push… | M1 | 🟡 |
| 提交选项：作者、Sign-off、GPG 签名、运行 Git hooks、清理 | M3 | ✅ |
| 提交前检查：Reformat、Optimize imports、Analyze code、Check TODO（IDE 特有，客户端只保留 hooks） | — | — |
| 工具栏：Refresh、Rollback、Show Diff、Shelve、Stash、Update | M2 | ✅ |
| Diff 预览（选中文件即预览） | M2 | ✅ |
| Rollback Changes 对话框（删除本地副本选项） | M2 | ✅（列出要回滚的文件（可取消勾选）、修改/新增/删除计数、“Delete local copies of added files”；暂存模式下未暂存的从 index 回滚；文件右键 Rollback…） |
| 添加到 VCS / 添加到 .gitignore | M2 | ✅（未版本化文件右键：Add to VCS、Add to .gitignore、Add to .git/info/exclude） |
| 提交完成通知 + Undo | M2 | ✅（Undo：soft reset 并恢复提交信息） |

### 3.1 Shelf / Stash
| 功能 | 阶段 | 状态 |
|---|---|---|
| Stash Changes 对话框（消息、Keep index） | M2 | ✅（含 Include untracked） |
| Stashes 列表：查看内容、Apply、Pop、Drop、Clear、Unstash as branch、Reinstate index | M2 | ✅（查看、Apply、Pop、Unstash As…（Pop、Reinstate index、As new branch）、Drop（确认）、Clear（确认）） |
| Shelf（IntelliJ 特有补丁货架）：Shelve、Unshelve、Shelve silently、Recently deleted | M4 | ✅（Shelf 标签页；Unshelve、Unshelve and Keep、Rename、Delete、Restore、Import Patches；补丁文件 + refs/gitglass 保存） |

## 4. 分支弹窗（标题栏 VCS 组件 / `Ctrl+Shift+\``）

| 功能 | 阶段 | 状态 |
|---|---|---|
| 顶部动作：Update Project、Commit、Push、New Branch、Checkout Tag or Revision | M1 | ✅（Update Project…、Commit…、Push…、New Branch…、Checkout Tag or Revision…） |
| 搜索框（直接输入过滤） | M1 | ✅ |
| Recent、Local、Remote、Tags 分组；收藏星标 | M1 | ✅（Recent（reflog 最近签出）、Local、Remote、Tags 可折叠分组，Tags 默认折叠；收藏星标） |
| 当前分支标记、跟踪分支、ahead/behind 箭头 | M2 | ✅ |
| 分支子菜单：Checkout、New Branch from…、Checkout and Rebase onto Current、Compare with Current、Show Diff with Working Tree、Rebase Current onto Selected、Merge into Current、Pull into Current Using Rebase / Merge、Update、Push…、Rename…、Edit Tracking Branch、Delete | M2 | ✅ |
| 进行中的操作：Continue / Abort / Skip（rebase、merge、cherry-pick、revert） | M3 | ✅ |
| 多仓库：同步分支控制开关 | M5 | ⬜ |

## 5. 远程操作

| 功能 | 阶段 | 状态 |
|---|---|---|
| Fetch（全部 remote） | M2 | ✅ |
| Update Project（`Ctrl+T`）对话框：Merge / Rebase；Using Stash / Shelve | M2 | ✅（Merge / Rebase；Using Stash / Shelve，选择会记住） |
| 更新结果：Updated files 树、被更新的提交 Log 标签页 | M3 | ✅（通知：文件数 / 提交数；View Files 打开更新文件树，View Commits 打开 Update Info 日志标签页） |
| Pull 对话框：remote、分支、选项（--rebase、--ff-only、--no-ff、--squash、--no-commit） | M2 | ✅（Pull to <分支>：remote 下拉、分支输入+远程分支列表、--rebase/--ff-only/--no-ff/--squash/--no-commit/--no-verify，互斥项置灰，预览命令） |
| Push 对话框：每个仓库待推送的提交列表 + 变更树、目标分支可编辑（新分支标记）、Force push（--force-with-lease）、Push tags（All / Current branch）、Run hooks、Set upstream | M2 | ✅（左侧提交列表，右侧变更文件；点选提交只看该提交的文件） |
| 推送被拒：提示 Merge / Rebase 后重推，"自动更新"选项 | M3 | ✅ |
| 受保护分支禁止 force push | M3 | ✅ |
| Manage Remotes 对话框：添加/编辑/删除 | M2 | ✅（Git Remotes：列表、+ 添加 / − 删除 / 编辑（改名 + 改 URL），双击编辑） |
| 凭据：HTTPS 密码/Token 对话框、SSH passphrase、使用 credential helper | M2 | ✅（已配置的 credential helper 优先） |
| Clone 对话框：URL、目录、GitHub/GitLab 账号仓库列表 | M4 | 🟡（URL + 目录自动填充 + Test；GitHub/GitLab 账号仓库列表随 M6） |

## 6. Diff 与 Merge

| 功能 | 阶段 | 状态 |
|---|---|---|
| 双栏 Side-by-side / 统一 Unified 视图 | M2 | ✅ |
| 忽略空白：不忽略 / 行首尾 / 全部 / 仅空行 | M2 | 🟡（无“仅空行”） |
| 高亮：按词 / 按行 / 按字符 / 不高亮 | M2 | 🟡（无“按字符”） |
| 折叠未改动片段、同步滚动、上/下一处差异、跳到下一个文件 | M2 | 🟡（缺跳到下一个文件） |
| 右侧可编辑（工作区文件）、单个 chunk 回滚 / 应用 | M3 | 🟡（chunk Rollback / Stage / Unstage；右侧直接编辑未做） |
| 语法高亮（tree-sitter，与编辑器一致） | M2 | ⬜ |
| 二进制 / 图片对比 | M4 | ✅（并排显示两侧图片（PNG/JPEG/GIF/BMP/WebP/ICO），下方显示尺寸、格式、文件大小；非图片显示大小；新增/删除提示） |
| 冲突对话框：文件列表，Accept Yours / Accept Theirs / Merge… | M3 | ✅ |
| 三方合并工具：左（Yours）中（Result）右（Theirs）、魔棒应用非冲突改动、逐块接受、Resolve simple conflicts | M3 | ✅ |
| Compare with Branch… / Compare with Revision… / Compare two commits | M3 | ✅ |
| 分支比较视图（两个分支的提交差异 + 文件差异） | M3 | ✅（current..branch 提交列表 + Swap Branches + Show Files） |

## 7. 文件级功能

| 功能 | 阶段 | 状态 |
|---|---|---|
| 文件历史（Show History）：Log 标签页 + 该文件的 diff | M3 | ✅（Log 按路径过滤，--follow 跟踪重命名，History Up to Here） |
| 选中内容历史（Show History for Selection） | M4 | ✅（文件编辑器右键 Git → Show History for Selection，用 git log -L 打开 Log 标签页） |
| Annotate with Git Blame：作者/日期/提交，按时间着色，悬浮详情，Annotate previous revision | M3 | ✅ |
| 文件查看器里的变更标记（gutter）：点击看 diff、回滚 hunk、stage hunk | M4 | 🟡（文件编辑器：tree-sitter 语法高亮，新增/修改行底色、删除处细框；Rollback Lines（Ctrl+Alt+Z）；点击标记弹出 diff 未做） |
| Show Current Revision | M4 | ✅（编辑器右键 Git → Show Current Revision，在 Log 中选中最后修改该文件的提交） |

## 8. 交互式 Rebase 及其他操作

| 功能 | 阶段 | 状态 |
|---|---|---|
| 交互式 Rebase 对话框：pick / reword / edit / squash / fixup / drop，拖拽排序，右侧提交详情，Unite（合并多行） | M3 | ✅（拖拽排序、Ctrl/Cmd 多选后 Unite） |
| Rebase 对话框（git rebase 全部选项：--onto、--interactive、--rebase-merges、--keep-empty…） | M3 | ✅ |
| Merge 对话框（--no-ff、--ff-only、--squash、-m、--no-commit、--allow-unrelated-histories） | M3 | ✅ |
| Cherry-pick（多选）、冲突处理 | M3 | ✅ |
| Reset HEAD 对话框 | M2 | ✅ |
| Tag：新建（轻量/附注、指定提交）、删除、推送 | M2 | ✅（新建；分支弹窗 Tag 子菜单：Push to <remote>、Delete、Delete on <remote>） |
| 补丁：Create Patch / Apply Patch（含预览） | M4 | ✅（本地改动含未跟踪文件；Apply 先直接应用，失败时三方合并；剪贴板） |
| Worktree：列表、新建、删除、打开 | M5 | ✅（Git 工具窗口 Worktrees 标签页：列表、New Worktree…、Open、Open in New Window、Delete；菜单入口） |
| Submodule：识别、更新 | M5 | ⬜ |
| Git 控制台（Console 标签页）：所有执行过的 git 命令及输出 | M1 | 🟡 |

## 9. 设置（Settings › Version Control › Git）

| 设置项 | 阶段 | 状态 |
|---|---|---|
| Git 可执行文件路径 + Test 按钮 | M2 | ✅（设置里填写路径，留空为 PATH 中的 git；Test 显示 git 版本或错误） |
| 启用暂存区 | M2 | ✅ |
| 提交前警告 CRLF、警告 detached HEAD、大文件 | M3 | ✅（CRLF 支持 Fix and Commit） |
| Update method（Merge / Rebase）、Clean working tree using（Stash / Shelve） | M2 | ✅ |
| 推送被拒时自动更新、Force push 受保护分支列表 | M3 | ✅ |
| GPG 签名配置 | M4 | ✅（Commit Options → Configure…：列出 secret key，写入仓库 commit.gpgSign / user.signingKey） |
| 使用 credential helper | M2 | ✅（“Use credential helper”，默认开；关闭后只用 GitGlass 的凭据提示（askpass）） |
| 定期检查新的远程提交（incoming） | M4 | ✅（设置 › Update branch info：每 N 分钟（默认 10）后台静默 fetch；分支弹窗、Log 分支树、标题栏分支组件显示 ↓incoming ↑outgoing） |
| Directory mappings（多根） | M5 | ⬜ |
| Commit 设置：非模态提交、清理提交信息、右边距、首行长度 | M3 | ✅（首行长度计数，超限变红） |
| Log 设置：日期格式、显示/隐藏列 | M2 | ✅（View Options 的列、日期格式、引用、高亮、排序都保存在设置文件中） |

## 10. 托管平台集成（Android Studio 自带的 GitHub/GitLab 插件）

| 功能 | 阶段 | 状态 |
|---|---|---|
| 账号登录（OAuth / Token） | M6 | ⬜ |
| Pull Requests 工具窗口：列表、详情、diff 评论、审批、合并 | M6 | ⬜ |
| Share Project on GitHub、Create Gist | M6 | ⬜ |

## 阶段

- **M1 骨架**：玻璃窗口、标题栏、工具窗口布局、Log（分支树 + 提交图 + 详情）、Commit 面板基础提交、分支弹窗基础、Console。
- **M2 日常操作**：Diff 查看器、暂存区、Stash、Fetch/Pull/Push 对话框、分支全部动作、Reset/Tag、通知与快捷键、设置页。
- **M3 历史改写与冲突**：交互式 Rebase、Merge/Rebase/Cherry-pick 对话框、三方合并工具、Blame、文件历史、分支比较。
- **M4 进阶**：部分提交、多 changelist、Shelf、补丁、Clone、GPG。
- **M5 多仓库**：多根、Worktree、Submodule。
- **M6 托管平台**：GitHub/GitLab Pull Requests。
