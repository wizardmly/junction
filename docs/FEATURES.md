# Junction 功能清单：一比一复刻 Android Studio / IntelliJ 的 Git 客户端

对照基准：Android Studio（基于 IntelliJ Platform，新 UI）里的 Git 功能，也就是 git4idea + VCS Log + Commit 工具窗口 + Diff/Merge 工具。
状态：✅ 已实现　🟡 部分实现　⬜ 未开始。阶段：M1–M6，见文末。

## 1. 窗口外壳（新 UI）

| 功能 | 细节 | 阶段 | 状态 |
|---|---|---|---|
| 玻璃窗口 | macOS vibrancy / Windows Mica-Acrylic / Linux 半透明（KDE 模糊） | M1 | ✅（Windows 11 用 Mica，macOS 与 Linux 用模糊背景，配色半透明；Linux 是否模糊取决于桌面合成器，如 KDE） |
| 自定义标题栏 | 主菜单（汉堡按钮）、项目名组件、VCS 分支组件、右侧工具按钮 | M1 | ✅（汉堡主菜单带 File / View / Navigate / Git / Window / Help 子菜单，含 Tool Windows、Appearance、Recent Projects、Keyboard Shortcuts、About；右侧 Update / Commit / Push / History 加 Search Everywhere 与 Settings 按钮，提示里带各平台快捷键） |
| 项目组件 | 最近项目列表、打开、克隆、新建仓库 | M1/M4 | ✅（欢迎页 + 项目组件下拉：最近项目、Open、Get from VCS、Create Git Repository） |
| VCS 分支组件 | 当前分支名、进行中的操作（Rebasing/Merging/Cherry-picking）、点击弹出分支弹窗 | M1 | ✅（显示 “Merging master” 等；进行中时分支弹窗顶部有 Continue Rebase / Cherry-Pick / Revert、Skip Commit（rebase）、Abort） |
| 左/右/下 工具窗口条 | Commit、Git（Log/Console）、可拖拽、可隐藏、记住尺寸 | M1 | ✅（Project、Commit、Pull Requests、Changes、Git、Notifications；按钮可拖到左 / 右 / 下或在右键菜单 Move to，Hide；每侧同时开一个；Hide All（Ctrl+Shift+F12）、Hide Active（Shift+Esc）；布局和三侧尺寸写入设置） |
| 状态栏 | 分支、行分隔符、后台任务进度、通知 | M1 | ✅（面包屑 项目 › 路径、后台任务转圈、索引状态、行:列（点开 Go to Line）、LF/CRLF、UTF-8、缩进、只读锁、当前分支（点开分支弹窗）；通知进右侧 Notifications 工具窗口，未读时铃铛带点） |
| VCS 操作弹窗 | `Alt+\``（macOS `Ctrl+V`）快速操作列表 | M3 | ✅（数字键 1–9 快选、上下键、回车，显示各平台快捷键） |
| 通知气泡 | 操作结果、错误、可点击的动作（View、Undo、Show details） | M2 | ✅（右下角弹出，不挡冲突横幅；View Commit、Show Details、提交后 Undo；错误只摘 fatal/error 行） |
| 主题 | 亮/暗、跟随系统、Int UI 配色、紧凑模式 | M1 | ✅（Settings › Appearance：Dark / Light / Sync with OS，系统切换时跟着变（macOS / Windows 读系统设置；Linux 用 XDG portal 的 color-scheme，portal 尚未回答或不存在时看 GTK_THEME、GNOME 的 color-scheme / gtk-theme）；Compact mode 行高 20、工具栏 28；也在主菜单 View › Appearance） |
| 快捷键 | 与 IntelliJ 默认 keymap 一致（`Ctrl+K` 提交、`Ctrl+Shift+K` 推送、`Ctrl+T` 更新…） | M2 | ✅（Windows/Linux 用 IntelliJ 默认 keymap，macOS 用 macOS keymap：Ctrl/⌘+K、Ctrl/⌘+Shift+K、Ctrl/⌘+T、Alt/⌘+1/0/9、Ctrl+Alt+S/⌘,、Ctrl+G/⌘L、Ctrl+Alt+←→/⌘[ ]、Ctrl+Shift+F12、Shift+Esc、Ctrl+Enter / Ctrl+Alt+K 提交、F7、Ctrl+D、F4、Ctrl+Alt+Z 等；菜单显示 mac 符号；Help › Keyboard Shortcuts 列表） |

## 2. Git 工具窗口 › Log

### 2.1 分支面板（左侧，可隐藏）
| 功能 | 阶段 | 状态 |
|---|---|---|
| 树：HEAD (Current Branch)、Local、Remote（按 remote 分组）、Tags | M1 | ✅ |
| 收藏分支（星标）、置顶 | M2 | ✅（分支弹窗与 Log 分支面板：星标切换，收藏排在组内最前；默认收藏 main/master） |
| 按 `/` 分组为目录（feature/xxx） | M1 | ✅ |
| 分支搜索（直接输入） | M1 | ✅（Log 分支面板顶部搜索框，过滤并展开所有匹配、高亮匹配文字，Esc 清空；在树里直接输入即开始搜索；分支弹窗打开即聚焦搜索） |
| 单击定位到分支顶端、双击按分支过滤 Log | M1 | ✅（过滤后树的展开状态和选中项不变；分支顶端被当前过滤条件挡住时不选中列表外的提交，像 AS 一样提示 “… does not match active filters”，View and Reset Filters 清空过滤后定位；键盘：Enter 展开/折叠目录或按分支过滤，←/→ 折叠/展开或到父/子节点） |
| ahead/behind 指示（↑↓ 箭头） | M2 | ✅ |
| 工具栏：New Branch、Update Selected、Delete、Compare with Current、Show My Branches、Fetch、展开/折叠 | M2 | ✅（New Branch、Fetch、Update Selected、Delete、Compare with Current、Show My Branches（含我提交的分支）、展开/折叠、按分支过滤） |
| 多仓库根：Commit 和 Log 同时列出所有根 | M5 | ✅（像 AS 一样：Commit 窗口列出所有根的改动，Group By › Repository 每个根一个节点并显示其分支；勾选跨多个根的文件提交时每个根各提交一次（同一条信息），Stage/Unstage、Rollback、Add to VCS、Shelve、Diff 预览都按文件所在的根执行。Log 把所有根的提交按时间合并显示，左侧 Root 色条区分仓库（悬停显示根名），详情里显示 Root；Paths 过滤器多了 Roots 勾选项，可只看某几个根，Paths 路径可跨根；Branch 过滤按名字作用于所有有该分支的根；选中某个根的提交后，右键操作、详情、交互式 Rebase 等都在该根执行。分支弹窗 Repositories 区仍可切换当前根） |
| 右键菜单：与分支弹窗动作一致（见 §4） | M2 | ✅（与分支弹窗相同的动作 + Add to / Remove from Favorites） |

### 2.2 提交列表（中间）
| 功能 | 阶段 | 状态 |
|---|---|---|
| 提交图（彩色泳道、合并线、长边折叠为箭头） | M1 | ✅（跨 30 行以上的长边默认只画两端，以箭头收尾，点击箭头跳到另一端；View Options › Show Long Edges 画出整条边） |
| 列：Subject（含分支/标签标签）、Author、Date、Hash；列可显示/隐藏、可拖宽 | M1 | ✅（View Options › Show Columns：Author / Date / Hash 显示隐藏并记住；拖动列的左边缘改宽度，宽度记住） |
| 引用标签：本地分支、远程分支、标签、HEAD；左/右侧显示；紧凑引用视图 | M1 | ✅（本地/远程/标签/HEAD 标签；默认在右侧，与 IntelliJ 一致；View Options：Compact References View、Show References on the Left） |
| 虚拟滚动，几十万提交流畅 | M1 | ✅ |
| 分段加载（先加载最近的，滚动时加载更多） | M2 | ✅（先 1000 条，其余后台加载） |
| 多选（Shift/Ctrl） | M2 | ✅（多选后可 Cherry-Pick / Revert / 复制哈希；键盘：↑↓、PageUp/PageDown、Home/End，Shift+↑↓/Home/End 扩展选择，Ctrl+A 全选） |
| 高亮：我的提交（粗体）、合并提交（灰色）、当前分支提交、未合并到当前分支的提交 | M2 | ✅（View Options › Highlight：My Commits（粗体）、Merge Commits（灰色）、Current Branch（底色）、Not Merged into Current Branch（灰色）） |
| IntelliSort / 按拓扑 / 按日期排序 | M2 | ✅（View Options › Sort：IntelliSort（拓扑序）/ By Date（--date-order）） |
| 折叠/展开线性分支、显示长边 | M3 | ✅（View Options → Collapse Linear Branches，“⋯ N commits”点击展开；Show Long Edges） |
| 日期格式：相对时间 / 绝对时间 | M1 | ✅（View Options › Relative Dates：“5 minutes ago”；否则 Today/Yesterday/日期） |
| `Ctrl+F` 跳转到 hash / 分支 / 标签 | M2 | ✅（输入时在下方列出匹配的分支和标签，↑↓ 选择，Enter 跳转） |
| 多个 Log 标签页（从分支打开新标签） | M3 | ✅ |

### 2.3 过滤栏
| 功能 | 阶段 | 状态 |
|---|---|---|
| 文本/哈希搜索，选项：正则、区分大小写 | M1 | ✅（搜索框内 Cc（区分大小写）、.*（正则）开关；hash 前缀也能搜） |
| Branch 过滤（多选、收藏） | M1 | ✅（All、HEAD、Favorites、Select…（多选对话框）、Recent、本地/远程列表） |
| User 过滤（me、作者列表） | M2 | ✅（All、me（标签显示 “User: me”）、Select…（勾选作者或输入多个名字/邮箱）、Recent、作者列表（不随过滤变化）） |
| Date 过滤（最近 24h/7 天/自定义） | M2 | ✅（最近 24h / 7 天 / 30 天 / 1 年；Select… 自定义 From / To） |
| Paths 过滤（结构过滤：选择目录/文件） | M2 | ✅（Paths 下拉：All、Select Folders…（应用内的仓库目录树，勾选文件或目录，可多选）；文件夹不加 --follow） |
| 过滤历史记录 | M3 | ✅（Branch / User / Paths 下拉里的 Recent：最近 5 个过滤条件（当前会话）） |

### 2.4 提交详情 + 变更树（右侧）
| 功能 | 阶段 | 状态 |
|---|---|---|
| 变更文件树：按目录分组、按模块分组、扁平列表；文件状态颜色（新增绿、修改蓝、删除灰、重命名） | M1 | ✅（详情上方工具条：Expand All、Collapse All、View Options › Group By Directory / Module，都关时为扁平列表；状态颜色，重命名显示 from 旧路径） |
| 详情：完整提交信息、hash、作者/提交者、日期、包含该提交的分支、标签 | M1 | ✅（作者与提交者不同时显示提交者；超过 5 个分支时 Show all 展开全部） |
| 双击文件打开 Diff；Diff 预览（编辑器区或面板内） | M2 | ✅ |
| 多选提交时显示合并后的变更 | M3 | ✅ |
| 签名信息（GPG 验证） | M4 | ✅（详情面板显示 Verified / Bad / 无法校验 与签名者、key；GPG 与 SSH 签名） |
| 提交信息里的 issue 链接 / URL 可点击 | M3 | ✅（URL 和提交哈希） |

### 2.5 提交右键菜单
| 动作 | 阶段 | 状态 |
|---|---|---|
| Copy Revision Number（含 `Ctrl+C`） | M1 | ✅ |
| Create Patch… | M4 | ✅（多选时合并为一个补丁；保存到文件或剪贴板，可反向） |
| Cherry-Pick | M3 | ✅（冲突走进行中操作横幅；改动已在当前分支的（空）提交自动跳过，不留下进行中状态） |
| Checkout Revision | M2 | ✅ |
| Show Repository at Revision | M4 | ✅（提交右键 → 显示该版本的文件树，双击打开只读编辑器） |
| Compare with Local | M3 | ✅（提交详情里文件的右键菜单也有 Compare with Local / Compare Before with Local） |
| Reset Current Branch to Here…（Soft / Mixed / Hard / Keep） | M2 | ✅ |
| Revert Commit | M3 | ✅（冲突走进行中操作横幅） |
| Undo Commit（最新的未推送提交） | M2 | ✅ |
| Edit Commit Message…（reword） | M3 | ✅ |
| Fixup… / Squash Into… | M3 | ✅（预填 fixup!/squash! 提交信息，交互式 Rebase 自动归位） |
| Drop Commits | M3 | ✅（通知带 Undo：分支未再变化时恢复被删除的提交） |
| Squash Commits…（多选） | M3 | ✅ |
| Interactively Rebase from Here… | M3 | ✅ |
| Push All up to Here… | M3 | ✅（打开 Push 对话框，只列出到该提交为止的提交；没有 upstream 的分支也可用，推送后设置 upstream） |
| New Branch… / New Tag… | M2 | ✅ |
| Go to Child Commit / Go to Parent Commit | M2 | ✅（提交右键 Go to Child Commit / Go to Parent Commit） |
| Open on GitHub/GitLab | M5 | ✅（Log 提交右键、变更文件右键、文件编辑器 Git 菜单（带选中行）；支持 GitHub / GitLab / Bitbucket / Gitea 类主机） |

## 3. Commit 工具窗口（非模态提交）

| 功能 | 阶段 | 状态 |
|---|---|---|
| 变更树：Changes（changelist）、Unversioned Files、Ignored Files | M1 | ✅（Changes、Unversioned Files、Ignored Files 可在 View Options 显示） |
| 复选框选择要提交的文件；全选 | M1 | ✅（文件、目录、changelist 节点都有复选框，节点勾选即全选其下文件；部分勾选时显示三态 “—”，点它全选；点节点复选框不会折叠节点；新出现的改动（含提交 / Undo 后再次出现的路径、Add to VCS 后的文件）默认勾选；重命名作为一个改动提交 / 回滚 / unstage / shelve，旧路径的删除一起处理） |
| 分组：目录 / 模块 / 仓库；展开全部/折叠全部 | M2 | ✅（View Options › Group By Repository / Module / Directory，可组合，都关为扁平；模块按 build.gradle、Cargo.toml、CMakeLists.txt 等构建文件识别；展开全部、折叠全部） |
| 多个 Changelist：新建、移动文件到、设为活动 | M4 | ✅（New / Edit / Delete / Set Active；Move to Another Changelist；活动列表粗体，新改动进入活动列表） |
| Staging 模式（启用暂存区）：Staged / Unstaged 两棵树，Stage/Unstage 按钮 | M2 | ✅ |
| 部分提交：按 chunk / 按行勾选（diff 中的复选框） | M4 | ✅（changelist 模式：diff 中每个 chunk 有复选框，提交时用临时 index 只提交勾选的 chunk；暂存区模式按 chunk Stage / Unstage） |
| 提交信息编辑器：拼写检查、右边距线、首行长度提示、提交信息历史（`Ctrl+M`） | M1/M3 | ✅（等宽字体、右边距线、首行长度提示、Ctrl+M 历史；拼写检查：错词绿色波浪线，`Alt+Enter` 或右键错词弹出 Change to 建议和 Save to dictionary，内置 SCOWL 英文词表加开发常用词，用户词典存在配置目录 dictionary.txt，代码样式的词如 camelCase、路径、反引号内容不检查） |
| Amend 复选框（自动载入上次提交信息） | M1 | ✅（取消勾选时恢复原来输入的信息，载入的信息被改过则保留） |
| Commit / Commit and Push… | M1 | ✅（Ctrl+Enter 提交，Ctrl+Alt+K 提交并推送（macOS ⌘⏎ / ⌥⌘K），提交成功后弹出 Push 对话框；Amend 时为 Amend Commit / Amend Commit and Push…；信息为空时点 Commit 提示 “Specify commit message”；提交失败时保留提交信息、Amend 和作者） |
| 提交选项：作者、Sign-off、GPG 签名、运行 Git hooks、清理 | M3 | ✅（Author 输入框可输入空格） |
| 提交前检查：Reformat、Optimize imports、Analyze code、Check TODO（IDE 特有，客户端只保留 hooks） | — | — |
| 工具栏：Refresh、Rollback、Show Diff、Shelve、Stash、Update | M2 | ✅ |
| Diff 预览（选中文件即预览） | M2 | ✅（提交、回滚、stage 后刷新；“N of M” 和 Alt+←/→ 按变更树顺序；重命名文件与旧路径对比） |
| 变更树速搜（直接输入定位文件，Esc 关闭） | M2 | ✅ |
| 快捷键：Delete、Ctrl+Alt+Z、Alt+Shift+M、Ctrl+D、F4、Ctrl+Alt+A、Ctrl+Alt+H | M2 | ✅（Delete / Rollback / Move 作用于选中的文件或节点，不是已勾选的文件） |
| Rollback Changes 对话框（删除本地副本选项） | M2 | ✅（列出要回滚的文件（可取消勾选）、修改/新增/删除计数、“Delete local copies of added files”；暂存模式下未暂存的从 index 回滚；文件右键 Rollback…） |
| 添加到 VCS / 添加到 .gitignore | M2 | ✅（未版本化文件右键：Add to VCS、Add to .gitignore、Add to .git/info/exclude；已跟踪文件的菜单不显示 Add to .gitignore） |
| 提交完成通知 + Undo | M2 | ✅（Undo：soft reset 并恢复提交信息） |

### 3.1 Shelf / Stash
| 功能 | 阶段 | 状态 |
|---|---|---|
| Stash Changes 对话框（消息、Keep index） | M2 | ✅（含 Include untracked） |
| Stashes 列表：查看内容、Apply、Pop、Drop、Clear、Unstash as branch、Reinstate index | M2 | ✅（查看、Apply、Pop、Unstash As…（Pop、Reinstate index、As new branch）、Drop（确认）、Clear（确认）） |
| Shelf（IntelliJ 特有补丁货架）：Shelve、Unshelve、Shelve silently、Recently deleted | M4 | ✅（Shelf 标签页；Unshelve…（对话框：目标 changelist 或新建、“Remove successfully applied files from the shelf”）、Unshelve and Keep、Rename、Delete、Restore、Import Patches；Shelve Silently（Ctrl+Alt+H、右键菜单）；已添加的文件和重命名恢复为已添加状态；补丁文件 + refs/junction 保存） |

## 4. 分支弹窗（标题栏 VCS 组件 / `Ctrl+Shift+\``）

| 功能 | 阶段 | 状态 |
|---|---|---|
| 顶部动作：Update Project、Commit、Push、New Branch、Checkout Tag or Revision | M1 | ✅（Update Project…、Commit…、Push…、New Branch…、Checkout Tag or Revision…） |
| 搜索框（直接输入过滤） | M1 | ✅（输入时高亮第一个匹配；↑↓ 移动、Enter / → 展开分支动作、Enter 执行、← 返回分支；列表可用滚轮滚动；`Ctrl+Shift+\``（X11/Wayland 上报为 Ctrl+~ 也可）打开） |
| Recent、Local、Remote、Tags 分组；收藏星标 | M1 | ✅（Recent（reflog 最近签出）、Local、Remote、Tags 可折叠分组，Tags 默认折叠；收藏星标） |
| 当前分支标记、跟踪分支、ahead/behind 箭头 | M2 | ✅ |
| 分支子菜单：Checkout、New Branch from…、Checkout and Rebase onto Current、Compare with Current、Show Diff with Working Tree、Rebase Current onto Selected、Merge into Current、Pull into Current Using Rebase / Merge、Update、Push…、Rename…、Edit Tracking Branch、Delete | M2 | ✅（Checkout 遇到会被覆盖的本地改动时弹出 Git Checkout Problem：Smart Checkout / Force Checkout / Don't Checkout，Smart Checkout 恢复改动出现冲突时如实报告并保留 stash；签出远程分支而同名本地分支已存在时询问 Checkout Existing / Overwrite；删除未合并的分支时列出会丢失的提交并提供 Force Delete，删除后通知里有 Restore） |
| 进行中的操作：Continue / Abort / Skip（rebase、merge、cherry-pick、revert） | M3 | ✅（Abort 先确认；rebase 时标题栏显示 “Rebasing main”，即被 rebase 的分支名） |
| 多仓库：同步分支控制开关 | M5 | ✅（Execute branch operations on all roots：Checkout、New Branch 同步到所有根，设置会记住） |

## 5. 远程操作

| 功能 | 阶段 | 状态 |
|---|---|---|
| Fetch（全部 remote） | M2 | ✅ |
| Update Project（`Ctrl+T`）对话框：Merge / Rebase；Using Stash / Shelve | M2 | ✅（Merge / Rebase；Using Stash / Shelve，选择会记住；Don't show again 后 Ctrl+T 直接更新，设置 › Git 可恢复对话框；Merge 产生 “Merge remote-tracking branch 'origin/main'”；结果只统计收到的远程提交，View Commits 只列这些提交） |
| 更新结果：Updated files 树、被更新的提交 Log 标签页 | M3 | ✅（通知：文件数 / 提交数；View Files 打开更新文件树，View Commits 打开 Update Info 日志标签页） |
| Pull 对话框：remote、分支、选项（--rebase、--ff-only、--no-ff、--squash、--no-commit） | M2 | ✅（Pull to <分支>：remote 下拉、分支输入+远程分支列表、--rebase/--ff-only/--no-ff/--squash/--no-commit/--no-verify，互斥项置灰，预览命令） |
| Push 对话框：每个仓库待推送的提交列表 + 变更树、目标分支可编辑（新分支标记）、Force push（--force-with-lease）、Push tags（All / Current branch）、Run hooks、Set upstream | M2 | ✅（左侧提交列表，右侧变更文件；点选提交只看该提交的文件；改目标分支或从 remote 下拉换 remote 时实时重算提交和 New 标记；没有可推送内容时 Push 置灰；Force Push 像 AS 一样在 Push ▾ 下拉里，先弹出确认（Cancel 回到 Push 对话框），受保护分支时该项禁用） |
| 推送被拒：提示 Merge / Rebase 后重推，"自动更新"选项 | M3 | ✅（Push Rejected 对话框：Rebase / Merge / Cancel，默认按钮跟随设置的更新方式；“Remember the update method choice and silently update in future” 勾选后打开自动更新并记住方式；更新后自动重推） |
| 受保护分支禁止 force push | M3 | ✅ |
| Manage Remotes 对话框：添加/编辑/删除 | M2 | ✅（Git Remotes：列表、+ 添加 / − 删除 / 编辑（改名 + 改 URL），双击编辑；新 URL 先用 ls-remote 校验，删除前确认） |
| 凭据：HTTPS 密码/Token 对话框、SSH passphrase、使用 credential helper | M2 | ✅（已配置的 credential helper 优先） |
| Clone 对话框：URL、目录、GitHub/GitLab 账号仓库列表 | M4 | ✅（URL + 目录自动填充 + Test，填 URL 前不检查目录；已登录 GitHub 账号的仓库列表可搜索，点选填入 URL，某个账号取不到时只在列表里给出该账号的错误行；目录已存在且非空或 URL 为空时 Clone 置灰；克隆完成后像 AS 一样问 Open Project：This Window / New Window / Cancel，欢迎页直接打开） |

## 6. Diff 与 Merge

| 功能 | 阶段 | 状态 |
|---|---|---|
| 双栏 Side-by-side / 统一 Unified 视图 | M2 | ✅（双栏按 AS 重做：两栏只显示各自的行，gutter 镜像贴着中间分隔条，分隔条画连接块，左 gutter `>>` 回滚、右 gutter 包含复选框，标题条锁图标 + 版本 + 路径，error stripe 可点可拖；单栏也可编辑、有语法高亮，删除行只读显示在新增行上方，两列行号，gutter 有 Revert × 和包含复选框；逐项对照见 [DIFF_MERGE.md](DIFF_MERGE.md)） |
| 忽略空白：不忽略 / 行首尾 / 全部 / 全部加空行 | M2 | ✅ |
| 高亮：按词 / 按行 / 按字符 / 不高亮 | M2 | ✅（不高亮时连接块、`>>` 和复选框也不显示） |
| 折叠未改动片段、同步滚动、上/下一处差异、Jump to Source (F4)、跳到下一个文件 | M2 | ✅（F7 / Shift+F7 从光标所在行找下一处 / 上一处并移动光标；F4 打开光标所在行（另一侧的行按 diff 换算），和分支 / 版本 / 剪贴板比较时也能用；Alt+← / Alt+→、F7 到末尾进入下一个文件） |
| 右侧可编辑（工作区文件）、单个 chunk 回滚 / 应用 | M3 | ✅（双栏右侧直接编辑：光标、选择、拖选、双击选词、Smart Home、Ctrl+D/Y、Tab/Shift+Tab、输入法、撤销重做、复制粘贴，改完实时重算并自动保存；`>>` 在缓冲区里回滚可撤销，按住 Ctrl 变 Append（点击时也读取鼠标事件的 Ctrl）；部分包含的变更块和「全部包含」复选框显示三态「−」；右键 Compare with Clipboard；暂存区模式 chunk Stage / Unstage；单栏编辑未做） |
| 语法高亮（tree-sitter，与编辑器一致） | M2 | ✅（双栏、单栏、Merge 都有） |
| 所有支持语言都有语法高亮，配色与 AS 一致 | M9 | ✅（C、C++、Objective-C、Rust、Go、Java、Kotlin、Swift、Dart、V（.v/.vsh）、JavaScript、TypeScript、TSX、Python，以及 XML（含 .iml、.plist、.svg）、HTML、CSS、JSON、YAML、TOML、Markdown、Gradle（Groovy / Kotlin DSL）、.properties、CMake、Makefile、Shell、proto、SQL、Lua、Ruby、PHP、C#、Scala、diff；配色取 AS 新 UI 的 Dark / Light 方案，int、i32 等内置类型按关键字着色；每种语言有单元测试） |
| 图片直接查看 | M9 | ✅（PNG、JPG、GIF、WebP、BMP、ICO、TIFF 在编辑器标签里显示，默认适应窗口，Zoom In / Zoom Out / Actual Size / Fit 按钮，标题显示“宽x高 格式 大小”；历史版本里的图片也能看） |
| 二进制 / 图片对比 | M4 | ✅（并排显示两侧图片（PNG/JPEG/GIF/BMP/WebP/ICO），下方显示尺寸、格式、文件大小；非图片显示大小；新增/删除提示） |
| 冲突对话框：文件列表，Accept Yours / Accept Theirs / Merge… | M3 | ✅（Merge / Rebase / Cherry-pick / Update 遇到冲突时自动弹出；Commit 面板预填 MERGE_MSG 或被变基提交的信息） |
| 三方合并工具：左（Yours）中（Result）右（Theirs）、魔棒应用非冲突改动、逐块接受、Resolve simple conflicts | M3 | ✅（按 AS 重做：三个独立编辑器，结果栏从 base 开始、可直接编辑并可撤销，两条分隔条连接块，`>>` `×` / `<<` `×` 紧贴分隔条，第二侧自动 Append，冲突红色，魔棒逐词合并简单冲突，工具栏 Apply Non-Conflicting（左/全部/右）、Resolve Simple Conflicts、同步滚动；有未解决变更时 Apply 先确认；Compare Contents 下拉可把 Left / Right / Result 与 Base 或彼此对比；栏标题写分支名（Changes from main / Changes from feature）；F7 / Shift+F7 从光标找未解决的变更并移动光标；改过内容后 Cancel 先确认放弃，然后回到冲突对话框） |
| Compare with Branch… / Compare with Revision… / Compare two commits | M3 | ✅（Compare with Revision… 列出该文件的历史版本（跟随重命名，可输入过滤，↑↓ 选择）；Compare with Branch… 是分支 / 标签 / 版本输入框加分支列表；重命名文件的 diff 标题两侧各显示自己的路径） |
| 分支比较视图（两个分支的提交差异 + 文件差异） | M3 | ✅（current..branch 提交列表 + Swap Branches + Show Files） |

## 7. 文件级功能

| 功能 | 阶段 | 状态 |
|---|---|---|
| 文件历史（Show History）：Log 标签页 + 该文件的 diff | M3 | ✅（Log 按路径过滤，--follow 跟踪重命名，History Up to Here） |
| 选中内容历史（Show History for Selection） | M4 | ✅（文件编辑器右键 Git → Show History for Selection，用 git log -L 打开 Log 标签页） |
| Annotate with Git Blame：在编辑器 gutter 里逐行显示日期/作者（View 子菜单可加提交哈希），按时间着色，悬浮详情，跟随未保存的编辑；右键 Annotate Revision / Annotate Previous Revision（新增该文件的提交上置灰）/ Show Diff / Select in Git Log | M3 | ✅ |
| 文件查看器里的变更标记（gutter）：点击看 diff、回滚 hunk、stage hunk | M4 | ✅（行号旁的 gutter 竖条：新增绿、修改蓝、删除处灰色楔形；点击弹出 AS 式变更弹窗：上一处 / 下一处（循环）、Rollback（可撤销）、Show Diff、Copy、Stage（只把这块写入 index，其它已暂存内容保留），下方显示 HEAD 里的原内容；Esc 或点外面关闭，打字时自动关闭；Rollback Lines（Ctrl+Alt+Z）；未版本控制的文件像 AS 一样没有变更标记；右键行号 gutter 是 gutter 菜单：Annotate with Git Blame / Close Annotations、Show Line Numbers） |
| Show Current Revision | M4 | ✅（编辑器右键 Git → Show Current Revision，在 Log 中选中最后修改该文件的提交） |

## 8. 交互式 Rebase 及其他操作

| 功能 | 阶段 | 状态 |
|---|---|---|
| 交互式 Rebase 对话框：pick / reword / edit / squash / fixup / drop，拖拽排序，右侧提交详情，Unite（合并多行） | M3 | ✅（与 AS 对齐：Pick、Stop to Edit、Reword、Squash、Fixup、Drop 作用于所有选中行；单击 / Ctrl·Cmd 单击 / Shift 单击多选；选中多行时 Squash、Fixup 变成 Unite，把选中的提交移到最旧那条上方合成一个，Squash 合并所有提交信息；可一次拖动多行，蓝线指示落点；左侧图形列显示操作（合并的提交用色线连到目标，丢弃的提交离开主线）；双击行即 Reword 并聚焦信息框；快捷键 Alt+P / E / R / S / F / D、Delete、Alt+↑ / ↓ 移动、↑ / ↓ 选择） |
| Rebase 对话框（git rebase 全部选项：--onto、--interactive、--rebase-merges、--keep-empty…） | M3 | ✅ |
| Merge 对话框（--no-ff、--ff-only、--squash、-m、--no-commit、--allow-unrelated-histories） | M3 | ✅ |
| Cherry-pick（多选）、冲突处理 | M3 | ✅ |
| Reset HEAD 对话框 | M2 | ✅ |
| Tag：新建（轻量/附注、指定提交）、删除、推送 | M2 | ✅（新建；分支弹窗 Tag 子菜单：Push to <remote>、Delete、Delete on <remote>） |
| 补丁：Create Patch / Apply Patch（含预览） | M4 | ✅（本地改动含未跟踪文件；Apply 先直接应用，失败时三方合并；像 AS 一样把补丁新建的文件加入 Git，改动放进 “Add to changelist” 选的变更列表（暂存区模式下无此项）；剪贴板） |
| Worktree：列表、新建、删除、打开 | M5 | ✅（Git 工具窗口 Worktrees 标签页：列表、New Worktree…、Open、Open in New Window、Delete；菜单入口） |
| Submodule：识别、更新 | M5 | ✅（Submodules 标签页：状态、Update / Update All / Sync、打开或新窗口打开；diff 显示 Subproject commit（含 -dirty）；子模块图标；Update Project 时跟随更新） |
| Git 控制台（Console 标签页）：执行过的 git 命令及输出 | M1 | ✅（每条 “时:分:秒.毫秒: [仓库] git …”，下面是输出，错误红色；像 AS 一样只列出做事的命令，刷新用的只读查询不列；格式串里的控制字符显示为 %x1e；左侧 Soft-Wrap、Scroll to the End、Clear All） |

## 9. 设置（Settings › Version Control › Git）

| 设置项 | 阶段 | 状态 |
|---|---|---|
| Settings 对话框结构 | M2 | ✅（左侧搜索框 + 设置树：Appearance & Behavior › Appearance、Keymap、Editor › General / Font、Version Control › Commit / Directory Mappings / Git / GitHub / GitLab、Languages & Frameworks；搜索按页名和选项文字过滤并高亮命中项；Cancel / Apply / OK） |
| Keymap 页 | M2 | ✅（列出主要动作及快捷键，可 Change（按下新快捷键）、Remove、Reset，冲突时提示；改动存 keymap.conf，启动时覆盖默认绑定） |
| Editor 页 | M2 | ✅（General：软换行、行号、空白、缩进线，作用于文件编辑器和 diff / merge 查看器，改动即时生效；Font：字号，作用于编辑器、diff、merge，行高 1.6 倍） |
| Git 可执行文件路径 + Test 按钮 | M2 | ✅（设置里填写路径，留空为 PATH 中的 git；Test 显示 git 版本或错误） |
| 启用暂存区 | M2 | ✅ |
| 提交前警告 CRLF、警告 detached HEAD、大文件 | M3 | ✅（CRLF 的 Fix and Commit 像 AS 一样写全局 core.autocrlf；detached HEAD 用 AS 的提示文字；大文件：Settings › Version Control › Commit 的 “Warn about files larger than N MB”，默认 50 MB，可关闭） |
| Cherry-pick 后缀、Commit and Push 时显示 Push 对话框、在所有根上执行分支操作 | M3 | ✅（“Add the 'cherry picked from <hash>' suffix…”：所选提交已在受保护分支的远程分支上时加 -x；“Show Push dialog for Commit and Push” 及 “Show only for commits to protected branches”，关闭时直接推送当前分支；“Execute branch operations on all roots”） |
| Update method（Merge / Rebase）、Clean working tree using（Stash / Shelve） | M2 | ✅ |
| 推送被拒时自动更新、Force push 受保护分支列表 | M3 | ✅ |
| GPG 签名配置 | M4 | ✅（Commit Options → Configure…：列出 secret key，写入仓库 commit.gpgSign / user.signingKey） |
| 使用 credential helper | M2 | ✅（“Use credential helper”，默认开；关闭后只用 Junction 的凭据提示（askpass）；主机有已登录的 GitHub / GitLab 账号时 askpass 直接回答账号名和 token，不弹窗） |
| 定期检查新的远程提交（incoming） | M4 | ✅（设置 › Update branch info：每 N 分钟（默认 10）后台静默 fetch；分支弹窗、Log 分支树、标题栏分支组件显示 ↓incoming ↑outgoing） |
| Directory mappings（多根） | M5 | ✅（Settings › Version Control › Directory Mappings，Git 菜单也可打开；自动检测嵌套仓库和已初始化子模块，嵌套仓库不再显示为未版本控制目录；Add Root / Remove / Restore；Update Project 更新所有根） |
| Commit 设置：非模态提交、清理提交信息、右边距、首行长度 | M3 | ✅（“Use non-modal commit interface” 关闭后 Ctrl+K 打开模态 Commit Changes 对话框，像 AS 一样没有 Commit 工具窗口（stripe 按钮隐藏），Git 工具窗口多出 Local Changes / Shelf / Stash 标签，Alt+0 打开 Local Changes；Run Git hooks / Sign-off / Clean up；首行长度计数，超限变红） |
| Log 设置：日期格式、显示/隐藏列 | M2 | ✅（View Options 的列、日期格式、引用、高亮、排序都保存在设置文件中） |

## 10. 托管平台集成（Android Studio 自带的 GitHub/GitLab 插件）

| 功能 | 阶段 | 状态 |
|---|---|---|
| 账号登录（OAuth / Token） | M6 | 🟡（Token 登录，支持 github.com、GitHub Enterprise、gitlab.com 与自建 GitLab（账号对话框选 GitHub / GitLab）；账号存 accounts.json（权限 600）；OAuth 浏览器登录需注册 OAuth App，未做） |
| Pull Requests 工具窗口：列表、详情、diff 评论、审批、合并 | M6 | ✅（列表 + Open/Closed/All + 搜索；详情：描述、标签、文件，本地 diff（fetch refs/pull/N/head）；Timeline：评论、审查、行评论展示，发评论；Approve / Request Changes（正文为空时提示）；Merge… / Squash and Merge… 先编辑合并提交信息，Rebase and Merge… 先确认；Checkout；Create Pull Request（同一分支已有 PR 时提示并不重复创建）；列表另有 Author / Label / Assignee / Review 筛选，详情显示彩色标签、Reviewers、Assignees；PR diff 点新侧行号加行评论，已有评论以内联讨论串显示在行下方，可 Reply。GitLab 仓库显示为 Merge Requests 工具窗口：MR 列表、详情、文件 diff、Timeline（含系统消息）、评论与行评论（discussion）、Approve、Merge / Squash / Rebase、Checkout（merge-requests/N/head）、Create Merge Request（Draft 前缀），编号显示为 !N；MR 列表筛选 Author / Label / Assignee / Reviewer，Review 菜单按审批查询（Not approved / Approved / Approved by you / Review requested from you；GitLab API 无 Changes requested 查询）） |
| Share Project on GitHub、Create Gist | M6 | ✅（Git › GitHub / GitLab 菜单：Share Project、Create Pull Request、View Pull Requests、Open on GitHub、Create Gist、Manage Accounts；Share：建仓库、加 remote、无提交时先选初始提交的文件和提交信息、push -u；Gist：编辑器选区/整文件或 Commit 面板文件，Secret / 打开浏览器 / 复制 URL） |

## 11. 代码索引与跳转（M7）

内置索引（tree-sitter）始终可用，语言服务器（LSP）装了就叠加使用。跨语言桥接由自有索引解析。

| 功能 | 阶段 | 状态 |
|---|---|---|
| 符号索引：Kotlin、Java、C、C++、Objective-C、Rust、Swift、Dart、Go、V、JavaScript、TypeScript、Python | M7 | ✅（tree-sitter 解析；git ls-files 列表；按大小与修改时间增量更新，多线程；缓存在 .git/junction/index.json；保存文件立即重建该文件；状态栏显示进度与“N files, M symbols indexed”） |
| 跨语言桥接 | M7 | ✅（JNI：Java/Kotlin `native` ↔ C/C++ `Java_包_类_方法`（含 _1 等转义）与 RegisterNatives；Dart FFI `lookupFunction`/`@Native` ↔ C/Rust `extern "C"`/`#[no_mangle]`；Rust `extern "C"` 声明 ↔ C 实现；Swift `@_cdecl`/`@objc` ↔ ObjC/C；Go `//export` 与 cgo `C.name`、V `C.name` ↔ C；C 头文件原型 → 跨 ABI 的实现） |
| Go to Declaration（Ctrl+B / Ctrl+点击 / F12） | M7 | ✅（顺序：桥接 → LSP → 索引；多个目标弹出 Choose Declaration 列表（↑↓ 选择、Enter 跳转，右侧显示位置，库文件显示“库名 › 路径:行”）；光标在声明本身上时改为列出用法（只有一处直接跳过去）；找不到时给信息提示；目标不在视野内时滚到编辑器中部；Ctrl 悬停下划线） |
| Quick Documentation（悬停） | M7 | ✅（LSP hover，否则显示声明行与位置） |
| Find Usages（Alt+F7） | M7 | ✅（Find 工具窗口，按类别 › 文件 › 行分组；索引 + git grep，注释和字符串字面量里的文本不算（Kotlin / Dart 的 `$x`、Swift 的 `\(x)` 插值算代码），LSP references 标记“verified”；桥接两侧都列出；结果行高亮命中词，同名文件显示目录，右侧 Preview Source 预览选中结果；编辑器里也可 Ctrl+Alt+↓ / ↑ 跳到下一处 / 上一处） |
| Go to File / Class / Symbol（Ctrl+Shift+N / Ctrl+N / Ctrl+Alt+Shift+N） | M7 | ✅（打开 Search Everywhere 对应标签，见第 12 节） |
| Navigate Back / Forward（Ctrl+Alt+← / →） | M7 | ✅ |
| Project 工具窗口（Alt+1）、Select in Project View（Alt+F1） | M7 | ✅ |
| Project 窗口视图切换：Project（根节点带绝对路径）/ Project Files / Production / Tests / Open Files / Changed Files | M8 | ✅（Open Files 即当前打开的标签） |
| Project 窗口头部：New（File / Directory）、Select Opened File（准星）、Expand All、Collapse All | M8 | ✅ |
| ⋮ › Behavior：Always Select Opened File、Open Files with Single Click（默认双击打开） | M8 | ✅（设置持久化） |
| ⋮ › Appearance：Show Excluded Files（Git 忽略的文件橄榄色显示，展开时按需读盘）、Compact Middle Packages | M8 | ✅ |
| ⋮ › Sort By：Name / Type / Modification Time、Folders Always on Top | M8 | ✅ |
| Speed Search（Ctrl+F 或直接输入）：高亮匹配、↑↓ 跳到上/下一个匹配、Esc 关闭 | M8 | ✅ |
| 文件按 VCS 状态着色（修改蓝、新增绿、未跟踪红），含已修改文件的目录也着色 | M8 | ✅ |
| 键盘导航：↑↓ / ← 收起或回到父目录 / → 展开或进入 / Enter、F4 打开 / Home / End / PgUp / PgDn | M8 | ✅ |
| ⋮ › Edit Scopes、Group Tabs、View Mode、Move to、Resize、Remove from Sidebar、Help；Android / Packages 视图 | — | ❌ 不适用（IDE 窗口布局与 Android 模块模型，Junction 没有） |
| External Libraries：依赖库与 SDK 源码进入索引（跳转、悬停、Go to Class / File / Symbol） | M9 | ✅（Cargo：`cargo metadata` 按本机平台解析，含 Rust std/core/alloc；Go：go.mod + GOMODCACHE、Go SDK；Dart/Flutter：package_config.json、Dart SDK；Gradle：build.gradle / libs.versions.toml 声明的依赖，沿 .module / POM 传递，解压 Gradle 缓存里的 -sources.jar；Android SDK `sources/android-<compileSdk>`；JDK src.zip（含 Android Studio 自带 JBR）；C/C++/ObjC：从项目 #include 出发沿 NDK sysroot、compile_commands.json、编译器搜索路径、Apple SDK framework 解析头文件；SwiftPM / Xcode DerivedData / CocoaPods / Carthage、SDK 的 .swiftinterface；node_modules（优先 .d.ts）；Python venv / 标准库；V vlib / ~/.vmodules。索引按库缓存在用户缓存目录 Junction/libraries，跨项目共享；生成的超大绑定库只保留类型和函数） |
| 库源码只读打开 | M9 | ✅（标题与状态栏面包屑显示“External Libraries › 库名 › 路径”；库文件里可继续跳转；Select Opened File 定位到 External Libraries 下） |
| 跳转排序 | M9 | ✅（项目符号优先；库符号按文件的 import / use / #include（含被包含头文件）匹配；`x.member` 按 x 的声明类型（局部声明、字段声明行、返回类型）只留该类型的成员；类型位置（`name: String`、`List<T>`、`Foo x`）和非调用处跳到类本身，不列构造函数；Java / Kotlin 的 `import a.b.Name` 只认包 a.b 里的 Name，默认导入的 java.lang / kotlin 不含子包） |
| Project 窗口 External Libraries 节点 | M9 | ✅（与 AS 一致：Project 视图底部，SDK 在前；库名“Gradle: group:artifact:version@aar”、“< Android API 34, extension level 7 Platform >”（灰字 SDK 路径）、“< jbr-21 >”；展开为“xxx.jar  library root”，包名用点号合并（androidx.collection），JVM 源码按类显示（类/接口/枚举/注解/object/抽象类图标，Kotlin 角标，嵌套类可展开，顶层函数显示为 XxxKt），库内区域浅黄底；库文件的状态栏面包屑为“xxx.jar › 包 › 类”） |
| Project 窗口图标与 AS 新 UI 一致 | M9 | ✅（模块文件夹带蓝/绿角标，排除目录橙色文件夹 + 浅黄底，被忽略文件棕色字；Gradle 大象（.kts 带 Kotlin 角标）、Kotlin、Java 类、Markdown “M↓”、properties 齿轮、.gitignore、脚本、文本、XML、JSON、图片各自的图标） |
| 全应用图标与按钮样式与 AS 新 UI 一致 | M9 | ✅（图标直接用 IntelliJ 官方新 UI SVG（Apache 2.0，assets/as-icons），按原色分层渲染、浅色/深色各一套：工具窗口栏、各工具栏、Git/Log/Commit/Diff/Merge、弹窗、搜索、状态栏；工具栏按钮 22px、图标 16px，工具窗口栏图标 20px；悬停/按下底色取 AS ActionButton 的颜色，悬停不改图标颜色） |
| 语言服务器 | M7 | ✅（clangd、rust-analyzer、gopls、jdtls、kotlin-lsp、sourcekit-lsp、dart、v-analyzer、typescript-language-server、pyright/pylsp；按最近的 Cargo.toml / go.mod / pubspec.yaml / settings.gradle 等子项目分别启动；打开文件即预热；服务器忙时 0.6 秒内退回索引结果） |
| 设置 › Languages & Frameworks | M7 | ✅（总开关；每种语言可填自定义命令或 off，显示 Installed / Not found / Index only） |

## 12. 搜索（M8）

对齐 Android Studio / IntelliJ 的 Find 系列。

| 功能 | 阶段 | 状态 |
|---|---|---|
| Find in Files（Ctrl+Shift+F） | M8 | ✅（浮动弹窗：Cc / W / .* 开关（Alt+C / W / X）；File mask 带常用掩码下拉，支持 `!` 排除；过滤器：Anywhere / In Comments / In String Literals / Except …；In Project / Module（含构建文件的目录）/ Directory（可递归、浏览）/ Scope（Project Files、Production、Test、Open Files、Current File、Recently Viewed、Recently Changed、Local Changes）；结果行高亮命中、右侧文件名和行号，下方只读预览；Enter 打开，Pin 后不关闭；选中文字或光标处单词自动填入；“N matches in M files”） |
| Replace in Files（Ctrl+Shift+R） | M8 | ✅（替换框；Replace 替换当前行，Replace All 确认后全部替换；正则 `$1` 分组；替换后重建索引、刷新 Commit 列表、重载未修改的编辑器） |
| Open in Find Window（Ctrl+Enter） | M8 | ✅（Find 工具窗口多标签；Found Occurrences › 目录 › 文件 › 行；Group by Directory、全部展开/折叠、上一处/下一处（Ctrl+Alt+↑/↓）、Exclude（Delete）、Rerun；替换标签有 Replace Selected / Replace All；“Open results in new tab”） |
| Search Everywhere（双击 Shift） | M8 | ✅（All / Classes / Files / Symbols / Actions / Text 六个标签，Tab 切换；All 每组最多 6 条带“… more”；`/` 列出标签命令；`File.kt:12:3` 跳到行列；Include non-project items（All / Classes / Files / Symbols；默认只列项目内的项，勾选或再按一次快捷键才加入库，项目里没有匹配时也列库；Files 还包含被 git 忽略的文件；每次打开重置）；预览（Alt+P）；按语言 / 分组过滤；Open in Find Tool Window。双击 Shift 已实现，走 gpui 的单独修饰键绑定；Linux 测试环境（Xvfb）收不到单独修饰键事件，所以只实测了 Ctrl+N 等入口） |
| Find Action（Ctrl+Shift+A） | M8 | ✅（Git、Navigate、Window 三组命令，显示快捷键） |
| 匹配规则 | M8 | ✅（IntelliJ 式：查询字符按顺序，每个字符要么紧接上一个命中，要么在词首（驼峰、`_`、`.`、`/`、`-` 之后）；子串也算命中但排在后面） |
| 编辑器内 Find / Replace（Ctrl+F / Ctrl+R），Find Next / Previous（F3 / Shift+F3） | M8 | ✅（编辑器上方查找栏：Cc / W / .* 开关（Alt+C / W / X）、命中数 “3/17”、无结果或正则错误时红色、所有命中高亮当前项加框；↑↓、Enter / Shift+Enter、F3 / Shift+F3；Replace 行：Replace、Replace All（正则支持 `$1`）、Exclude；Esc 关闭（焦点回到编辑器后在编辑器里按 Esc 也关闭）；F3 无查询时用光标处单词） |
| Recent Files（Ctrl+E） | M8 | ✅（上一个文件排第一，Enter 即切回；↑↓ 选择；可输入过滤；库文件显示库名与目录） |
| File Structure（Ctrl+F12） | M8 | ✅（当前文件的声明，按容器缩进，可过滤，↑↓ 选择、Enter 跳转） |
| Go to Line:Column（Ctrl+G） | M8 | ✅（预填当前“行:列”并全选，直接输入即替换；`行[:列]`，记入 Back 历史） |
| 文件编辑器：语法高亮、撤销重做、删除行 | M8 | ✅（tree-sitter 高亮 Kotlin、Java、C、C++、Rust、Go、Swift、Dart、Python、JS/TS、TOML、YAML 等；IntelliJ keymap：Ctrl+Z 撤销、Ctrl+Shift+Z 重做、Ctrl+Y 删除行（macOS：⌘Z / ⇧⌘Z / ⌘⌫）） |

## 13. 编辑器标签页（M8）

| 功能 | 阶段 | 状态 |
|---|---|---|
| 每个打开的文件一个标签；文件图标、VCS 状态着色、未保存标记 •；同名文件附带目录名 | M8 | ✅ |
| Diff、Merge、Pull Request 视图也作为标签显示，可切换、可关闭 | M8 | ✅ |
| 新标签开在当前标签右侧；关闭后激活左侧标签；关闭时自动保存 | M8 | ✅ |
| 标签上限 10 个，超出时关闭最久未用的、未固定且未修改的标签 | M8 | ✅ |
| 右键：Close / Close Other Tabs / Close Tabs to the Left / Right / Close All Tabs / Close All but Pinned / Pin Tab / Copy Path / Copy Path From Repository Root / Select in Project View / Reopen Closed Tab | M8 | ✅ |
| 固定标签排在最前并显示图钉；拖动标签调整顺序 | M8 | ✅ |
| 中键关闭；Ctrl+F4 关闭；Alt+← / Alt+→ 切换（macOS 为 Cmd+Shift+[ / ]） | M8 | ✅ |
| 分屏：Split Right / Down、Split and Move Right / Down、Open in / Move to Opposite Group、Unsplit；标签可拖到另一组；某组标签关完后自动合并 | M8 | ✅（两组；同一文件在两组各有一份编辑缓冲） |
| 预览标签（Project 窗口 ⋮ › Behavior › Enable Preview Tab） | M8 | ✅（单击文件在一个斜体标签里预览，下一次预览替换它；编辑、双击文件或双击标签后变成普通标签） |
| 多行标签、双击最大化编辑器 | — | ❌ 未做 |

## 阶段

- **M1 骨架**：玻璃窗口、标题栏、工具窗口布局、Log（分支树 + 提交图 + 详情）、Commit 面板基础提交、分支弹窗基础、Console。
- **M2 日常操作**：Diff 查看器、暂存区、Stash、Fetch/Pull/Push 对话框、分支全部动作、Reset/Tag、通知与快捷键、设置页。
- **M3 历史改写与冲突**：交互式 Rebase、Merge/Rebase/Cherry-pick 对话框、三方合并工具、Blame、文件历史、分支比较。
- **M4 进阶**：部分提交、多 changelist、Shelf、补丁、Clone、GPG。
- **M5 多仓库**：多根、Worktree、Submodule。
- **M6 托管平台**：GitHub/GitLab Pull Requests。
- **M7 代码索引**：多语言符号索引、跨语言桥接、LSP、Go to Declaration / Find Usages / Go to Symbol。
- **M8 搜索**：Find / Replace in Files、Search Everywhere、Find Action、Recent Files、File Structure、Go to Line。
