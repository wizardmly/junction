# GitGlass 功能清单：一比一复刻 Android Studio / IntelliJ 的 Git 客户端

对照基准：Android Studio（基于 IntelliJ Platform，新 UI）里的 Git 功能，也就是 git4idea + VCS Log + Commit 工具窗口 + Diff/Merge 工具。
状态：✅ 已实现　🟡 部分实现　⬜ 未开始。阶段：M1–M6，见文末。

## 1. 窗口外壳（新 UI）

| 功能 | 细节 | 阶段 | 状态 |
|---|---|---|---|
| 玻璃窗口 | macOS vibrancy / Windows Mica-Acrylic / Linux 半透明（KDE 模糊） | M1 | 🟡 |
| 自定义标题栏 | 主菜单（汉堡按钮）、项目名组件、VCS 分支组件、右侧工具按钮 | M1 | 🟡 |
| 项目组件 | 最近项目列表、打开、克隆、新建仓库 | M1/M4 | 🟡 |
| VCS 分支组件 | 当前分支名、进行中的操作（Rebasing/Merging/Cherry-picking）、点击弹出分支弹窗 | M1 | 🟡 |
| 左/右/下 工具窗口条 | Commit、Git（Log/Console）、可拖拽、可隐藏、记住尺寸 | M1 | 🟡 |
| 状态栏 | 分支、行分隔符、后台任务进度、通知 | M1 | 🟡 |
| VCS 操作弹窗 | `Alt+\``（macOS `Ctrl+V`）快速操作列表 | M3 | ⬜ |
| 通知气泡 | 操作结果、错误、可点击的动作（View、Undo、Show details） | M2 | 🟡（结果与错误） |
| 主题 | 亮/暗、跟随系统、Int UI 配色、紧凑模式 | M1 | 🟡 |
| 快捷键 | 与 IntelliJ 默认 keymap 一致（`Ctrl+K` 提交、`Ctrl+Shift+K` 推送、`Ctrl+T` 更新…） | M2 | 🟡（Ctrl+K、Ctrl+Shift+K、Ctrl+T、Ctrl+Shift+\`、Alt+9、F7/Shift+F7） |

## 2. Git 工具窗口 › Log

### 2.1 分支面板（左侧，可隐藏）
| 功能 | 阶段 | 状态 |
|---|---|---|
| 树：HEAD (Current Branch)、Local、Remote（按 remote 分组）、Tags | M1 | ✅ |
| 收藏分支（星标）、置顶 | M2 | ⬜ |
| 按 `/` 分组为目录（feature/xxx） | M1 | ✅ |
| 分支搜索（直接输入） | M1 | ⬜ |
| 单击定位到分支顶端、双击按分支过滤 Log | M1 | ✅ |
| ahead/behind 指示（↑↓ 箭头） | M2 | ✅ |
| 工具栏：New Branch、Update Selected、Delete、Compare with Current、Show My Branches、Fetch、展开/折叠 | M2 | 🟡（New Branch、Fetch、按分支过滤） |
| 多仓库根时按仓库分组 | M5 | ⬜ |
| 右键菜单：与分支弹窗动作一致（见 §4） | M2 | ⬜ |

### 2.2 提交列表（中间）
| 功能 | 阶段 | 状态 |
|---|---|---|
| 提交图（彩色泳道、合并线、长边折叠为箭头） | M1 | ✅ |
| 列：Subject（含分支/标签标签）、Author、Date、Hash；列可显示/隐藏、可拖宽 | M1 | 🟡 |
| 引用标签：本地分支、远程分支、标签、HEAD；左/右侧显示；紧凑引用视图 | M1 | 🟡 |
| 虚拟滚动，几十万提交流畅 | M1 | ✅ |
| 分段加载（先加载最近的，滚动时加载更多） | M2 | ⬜ |
| 多选（Shift/Ctrl） | M2 | ⬜ |
| 高亮：我的提交（粗体）、合并提交（灰色）、当前分支提交、未合并到当前分支的提交 | M2 | ⬜ |
| IntelliSort / 按拓扑 / 按日期排序 | M2 | ⬜ |
| 折叠/展开线性分支、显示长边 | M3 | ⬜ |
| 日期格式：相对时间 / 绝对时间 | M1 | 🟡 |
| `Ctrl+F` 跳转到 hash / 分支 / 标签 | M2 | ⬜ |
| 多个 Log 标签页（从分支打开新标签） | M3 | ⬜ |

### 2.3 过滤栏
| 功能 | 阶段 | 状态 |
|---|---|---|
| 文本/哈希搜索，选项：正则、区分大小写 | M1 | 🟡 |
| Branch 过滤（多选、收藏） | M1 | 🟡 |
| User 过滤（me、作者列表） | M2 | ✅ |
| Date 过滤（最近 24h/7 天/自定义） | M2 | 🟡（无自定义） |
| Paths 过滤（结构过滤：选择目录/文件） | M2 | ⬜ |
| 过滤历史记录 | M3 | ⬜ |

### 2.4 提交详情 + 变更树（右侧）
| 功能 | 阶段 | 状态 |
|---|---|---|
| 变更文件树：按目录分组、按模块分组、扁平列表；文件状态颜色（新增绿、修改蓝、删除灰、重命名） | M1 | 🟡 |
| 详情：完整提交信息、hash、作者/提交者、日期、包含该提交的分支、标签 | M1 | 🟡 |
| 双击文件打开 Diff；Diff 预览（编辑器区或面板内） | M2 | ✅ |
| 多选提交时显示合并后的变更 | M3 | ⬜ |
| 签名信息（GPG 验证） | M4 | ⬜ |
| 提交信息里的 issue 链接 / URL 可点击 | M3 | ⬜ |

### 2.5 提交右键菜单
| 动作 | 阶段 | 状态 |
|---|---|---|
| Copy Revision Number（含 `Ctrl+C`） | M1 | ✅ |
| Create Patch… | M4 | ⬜ |
| Cherry-Pick | M3 | 🟡（无冲突处理） |
| Checkout Revision | M2 | ✅ |
| Show Repository at Revision | M4 | ⬜ |
| Compare with Local | M3 | ⬜ |
| Reset Current Branch to Here…（Soft / Mixed / Hard / Keep） | M2 | ✅ |
| Revert Commit | M3 | 🟡（无冲突处理） |
| Undo Commit（最新的未推送提交） | M2 | ✅ |
| Edit Commit Message…（reword） | M3 | ⬜ |
| Fixup… / Squash Into… | M3 | ⬜ |
| Drop Commits | M3 | ⬜ |
| Squash Commits…（多选） | M3 | ⬜ |
| Interactively Rebase from Here… | M3 | ⬜ |
| Push All up to Here… | M3 | ⬜ |
| New Branch… / New Tag… | M2 | ✅ |
| Go to Child Commit / Go to Parent Commit | M2 | 🟡（Parent） |
| Open on GitHub/GitLab | M5 | ⬜ |

## 3. Commit 工具窗口（非模态提交）

| 功能 | 阶段 | 状态 |
|---|---|---|
| 变更树：Changes（changelist）、Unversioned Files、Ignored Files | M1 | 🟡 |
| 复选框选择要提交的文件；全选 | M1 | 🟡 |
| 分组：目录 / 模块 / 仓库；展开全部/折叠全部 | M2 | ⬜ |
| 多个 Changelist：新建、移动文件到、设为活动 | M4 | ⬜ |
| Staging 模式（启用暂存区）：Staged / Unstaged 两棵树，Stage/Unstage 按钮 | M2 | ⬜ |
| 部分提交：按 chunk / 按行勾选（diff 中的复选框） | M4 | ⬜ |
| 提交信息编辑器：拼写检查、右边距线、首行长度提示、提交信息历史（`Ctrl+M`） | M1/M3 | 🟡 |
| Amend 复选框（自动载入上次提交信息） | M1 | ✅ |
| Commit / Commit and Push… | M1 | 🟡 |
| 提交选项：作者、Sign-off、GPG 签名、运行 Git hooks、清理 | M3 | ⬜ |
| 提交前检查：Reformat、Optimize imports、Analyze code、Check TODO（IDE 特有，客户端只保留 hooks） | — | — |
| 工具栏：Refresh、Rollback、Show Diff、Shelve、Stash、Update | M2 | 🟡（Refresh、Rollback、Show Diff） |
| Diff 预览（选中文件即预览） | M2 | ✅ |
| Rollback Changes 对话框（删除本地副本选项） | M2 | ⬜ |
| 添加到 VCS / 添加到 .gitignore | M2 | ⬜ |
| 提交完成通知 + Undo | M2 | 🟡（通知） |

### 3.1 Shelf / Stash
| 功能 | 阶段 | 状态 |
|---|---|---|
| Stash Changes 对话框（消息、Keep index） | M2 | ✅（含 Include untracked） |
| Stashes 列表：查看内容、Apply、Pop、Drop、Clear、Unstash as branch、Reinstate index | M2 | 🟡（查看、Apply、Pop、Drop） |
| Shelf（IntelliJ 特有补丁货架）：Shelve、Unshelve、Shelve silently、Recently deleted | M4 | ⬜ |

## 4. 分支弹窗（标题栏 VCS 组件 / `Ctrl+Shift+\``）

| 功能 | 阶段 | 状态 |
|---|---|---|
| 顶部动作：Update Project、Commit、Push、New Branch、Checkout Tag or Revision | M1 | 🟡 |
| 搜索框（直接输入过滤） | M1 | ✅ |
| Recent、Local、Remote、Tags 分组；收藏星标 | M1 | 🟡 |
| 当前分支标记、跟踪分支、ahead/behind 箭头 | M2 | ✅ |
| 分支子菜单：Checkout、New Branch from…、Checkout and Rebase onto Current、Compare with Current、Show Diff with Working Tree、Rebase Current onto Selected、Merge into Current、Pull into Current Using Rebase / Merge、Update、Push…、Rename…、Edit Tracking Branch、Delete | M2 | 🟡（缺 Show Diff、Edit Tracking） |
| 进行中的操作：Continue / Abort / Skip（rebase、merge、cherry-pick、revert） | M3 | ⬜ |
| 多仓库：同步分支控制开关 | M5 | ⬜ |

## 5. 远程操作

| 功能 | 阶段 | 状态 |
|---|---|---|
| Fetch（全部 remote） | M2 | ✅ |
| Update Project（`Ctrl+T`）对话框：Merge / Rebase；Using Stash / Shelve | M2 | 🟡（Merge / Rebase，自动 stash） |
| 更新结果：Updated files 树、被更新的提交 Log 标签页 | M3 | ⬜ |
| Pull 对话框：remote、分支、选项（--rebase、--ff-only、--no-ff、--squash、--no-commit） | M2 | ⬜ |
| Push 对话框：每个仓库待推送的提交列表 + 变更树、目标分支可编辑（新分支标记）、Force push（--force-with-lease）、Push tags（All / Current branch）、Run hooks、Set upstream | M2 | 🟡（无变更树） |
| 推送被拒：提示 Merge / Rebase 后重推，"自动更新"选项 | M3 | ⬜ |
| 受保护分支禁止 force push | M3 | ⬜ |
| Manage Remotes 对话框：添加/编辑/删除 | M2 | ⬜ |
| 凭据：HTTPS 密码/Token 对话框、SSH passphrase、使用 credential helper | M2 | ⬜ |
| Clone 对话框：URL、目录、GitHub/GitLab 账号仓库列表 | M4 | ⬜ |

## 6. Diff 与 Merge

| 功能 | 阶段 | 状态 |
|---|---|---|
| 双栏 Side-by-side / 统一 Unified 视图 | M2 | ✅ |
| 忽略空白：不忽略 / 行首尾 / 全部 / 仅空行 | M2 | 🟡（无“仅空行”） |
| 高亮：按词 / 按行 / 按字符 / 不高亮 | M2 | 🟡（无“按字符”） |
| 折叠未改动片段、同步滚动、上/下一处差异、跳到下一个文件 | M2 | 🟡（缺跳到下一个文件） |
| 右侧可编辑（工作区文件）、单个 chunk 回滚 / 应用 | M3 | ⬜ |
| 语法高亮（tree-sitter，与编辑器一致） | M2 | ⬜ |
| 二进制 / 图片对比 | M4 | ⬜ |
| 冲突对话框：文件列表，Accept Yours / Accept Theirs / Merge… | M3 | ⬜ |
| 三方合并工具：左（Yours）中（Result）右（Theirs）、魔棒应用非冲突改动、逐块接受、Resolve simple conflicts | M3 | ⬜ |
| Compare with Branch… / Compare with Revision… / Compare two commits | M3 | ⬜ |
| 分支比较视图（两个分支的提交差异 + 文件差异） | M3 | ⬜ |

## 7. 文件级功能

| 功能 | 阶段 | 状态 |
|---|---|---|
| 文件历史（Show History）：Log 标签页 + 该文件的 diff | M3 | ⬜ |
| 选中内容历史（Show History for Selection） | M4 | ⬜ |
| Annotate with Git Blame：作者/日期/提交，按时间着色，悬浮详情，Annotate previous revision | M3 | ⬜ |
| 文件查看器里的变更标记（gutter）：点击看 diff、回滚 hunk、stage hunk | M4 | ⬜ |
| Show Current Revision | M4 | ⬜ |

## 8. 交互式 Rebase 及其他操作

| 功能 | 阶段 | 状态 |
|---|---|---|
| 交互式 Rebase 对话框：pick / reword / edit / squash / fixup / drop，拖拽排序，右侧提交详情，Unite（合并多行） | M3 | ⬜ |
| Rebase 对话框（git rebase 全部选项：--onto、--interactive、--rebase-merges、--keep-empty…） | M3 | ⬜ |
| Merge 对话框（--no-ff、--ff-only、--squash、-m、--no-commit、--allow-unrelated-histories） | M3 | ⬜ |
| Cherry-pick（多选）、冲突处理 | M3 | ⬜ |
| Reset HEAD 对话框 | M2 | ✅ |
| Tag：新建（轻量/附注、指定提交）、删除、推送 | M2 | 🟡（新建） |
| 补丁：Create Patch / Apply Patch（含预览） | M4 | ⬜ |
| Worktree：列表、新建、删除、打开 | M5 | ⬜ |
| Submodule：识别、更新 | M5 | ⬜ |
| Git 控制台（Console 标签页）：所有执行过的 git 命令及输出 | M1 | 🟡 |

## 9. 设置（Settings › Version Control › Git）

| 设置项 | 阶段 | 状态 |
|---|---|---|
| Git 可执行文件路径 + Test 按钮 | M2 | ⬜ |
| 启用暂存区 | M2 | ⬜ |
| 提交前警告 CRLF、警告 detached HEAD、大文件 | M3 | ⬜ |
| Update method（Merge / Rebase）、Clean working tree using（Stash / Shelve） | M2 | ⬜ |
| 推送被拒时自动更新、Force push 受保护分支列表 | M3 | ⬜ |
| GPG 签名配置 | M4 | ⬜ |
| 使用 credential helper | M2 | ⬜ |
| 定期检查新的远程提交（incoming） | M4 | ⬜ |
| Directory mappings（多根） | M5 | ⬜ |
| Commit 设置：非模态提交、清理提交信息、右边距、首行长度 | M3 | ⬜ |
| Log 设置：日期格式、显示/隐藏列 | M2 | ⬜ |

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
