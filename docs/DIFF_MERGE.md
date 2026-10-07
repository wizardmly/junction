# Diff / Merge 查看器复刻规格

对照对象是 Android Studio（IntelliJ 平台）的 `SimpleDiffViewer`（双栏 diff）、`UnifiedDiffViewer`（单栏）和 `TextMergeViewer`（三栏 merge）。
这三者共用同一套部件：编辑器面板、中间分隔条（divider）、gutter 按钮、同步滚动、工具栏。所以先把这套部件做对，diff 和 merge 两边都能用上。

状态标记：✅ 已有，🟡 有但和 AS 不一致，❌ 没有。

## 1. 双栏 diff 的布局

| # | AS 的表现 | 现状 |
|---|---|---|
| 1.1 | 左右是**两个独立的编辑器**，各自只显示真实存在的行，不插空白行对齐 | ✅ |
| 1.2 | 左编辑器的 gutter（行号）在它的**右侧**，滚动条在最左边；右编辑器的 gutter 在左侧，滚动条在最右边。两个行号列紧贴中间分隔条 | ✅ |
| 1.3 | 中间分隔条（约 30px）画**梯形/斜线连接块**，把左边的变更范围连到右边对应的范围。颜色按类型区分：修改蓝、新增绿、删除灰。一边范围为空时，连接到对面的一条细线 | ✅ |
| 1.4 | 一边为空的变更（纯新增或纯删除），在空的那一侧画一条水平细线，标出插入位置 | ✅ |
| 1.5 | **同步滚动**：滚动一边时，另一边按变更映射跟着滚动，让视口中线附近的行对齐。工具栏有开关 | ✅ |
| 1.6 | 每栏顶部有标题条。左栏：只读锁图标 + 版本号（如 `d8e7e6f0`）+ 灰色路径；右栏：「全部包含」复选框 + `Current version`（或版本号） | ✅ |
| 1.7 | 每栏右侧滚动条上有 error stripe，按颜色标出所有变更的位置，点击可跳转 | ✅ |
| 1.8 | 「折叠未修改片段」：两栏各自折叠，折叠行可点开，两边同时展开；分隔条上连接两边的折叠块 | ✅ |
| 1.9 | 行高亮铺满整行，包括 gutter 区域 | ✅ |
| 1.10 | 单词级高亮：修改行内，新增的词深绿，删除的词深灰，修改的词深蓝（行底色是浅蓝） | ✅ |

## 2. Gutter 按钮（选择变更）

| # | AS 的表现 | 现状 |
|---|---|---|
| 2.1 | 右侧可编辑时（工作区文件），每个变更在**左编辑器 gutter 紧贴分隔条**显示 `>>`，点击把左边内容替换进右边（即回滚这块），tooltip 为 "Revert" / "Replace" | ✅ |
| 2.2 | 按住 Ctrl 时，`>>` 变成「Append」（插入左边内容，不删除右边） | ✅ |
| 2.3 | 左边也可编辑时，右 gutter 显示 `<<` | ❌ |
| 2.4 | 部分提交：每个变更在**右编辑器 gutter** 的变更首行显示复选框（包含进本次提交） | ✅ |
| 2.5 | 右栏标题的复选框控制全部变更；工具栏右侧显示「7 differences, 0 included」 | ✅ |
| 2.6 | 暂存区模式：Stage / Unstage / Rollback 的箭头同样在 gutter 里 | ✅ |
| 2.7 | 编辑器右键菜单：Revert Selected Changes、Include/Exclude Lines into Commit、Compare with Clipboard 等 | ❌ |
| 2.8 | 行级部分提交：可以只包含一个变更里的部分行（右键 Include Lines into Commit） | ❌ |

## 3. 可编辑

| # | AS 的表现 | 现状 |
|---|---|---|
| 3.1 | 右栏是工作区文件时可以直接编辑（光标、选择、输入法、撤销、复制粘贴、语法高亮），编辑后 diff 实时重算，自动保存回磁盘 | ✅ |
| 3.2 | 只读的一栏不能编辑，但可以选择、复制 | ✅ |
| 3.3 | 两栏都有语法高亮 | ✅ |

## 4. 工具栏（从左到右）

| # | AS 的表现 | 现状 |
|---|---|---|
| 4.1 | ↑ Previous Difference（Shift+F7）、↓ Next Difference（F7）；到末尾再按会提示跳到下一个文件 | ✅ / ❌ 跨文件 |
| 4.2 | ✎ Jump to Source（F4） | ✅ |
| 4.3 | ← / → Compare Previous / Next File（Alt+←/→）；文件列表按钮（选择要对比的文件） | ❌ |
| 4.4 | 查看器下拉：Side-by-side viewer / Unified viewer | ✅ |
| 4.5 | 空白下拉：Do not ignore / Trim whitespaces / Ignore whitespaces / Ignore whitespaces and empty lines | 🟡 缺最后一项 |
| 4.6 | 高亮下拉：Highlight words / Highlight lines / Highlight split changes / Highlight characters / Do not highlight | 🟡 缺两项 |
| 4.7 | 折叠未修改片段开关 | ✅ |
| 4.8 | 同步滚动开关 | ✅ |
| 4.9 | 齿轮菜单：上下文行数、显示行号、显示空白字符、显示缩进线、软换行、对齐变更（Align changes）、在编辑器标签页打开等 | ❌ |
| 4.10 | ? 帮助 | ❌ |
| 4.11 | 右侧统计：`N differences`，部分提交时为 `N differences, M included` | ✅ |

## 5. 单栏 diff（Unified）

| # | AS 的表现 | 现状 |
|---|---|---|
| 5.1 | 一个编辑器，删除行以只读块显示在新增行上方，两列行号 | ✅ |
| 5.2 | gutter 里每个变更有 Revert（×）和部分提交复选框 | 🟡 |
| 5.3 | 新内容一侧可编辑 | ❌ |

## 6. 三栏 Merge

| # | AS 的表现 | 现状 |
|---|---|---|
| 6.1 | 三个独立编辑器：左「Changes from 我方」只读，中「Result」可编辑，右「Changes from 对方」只读。标题带分支和版本 | ✅ |
| 6.2 | 两条分隔条，分别连接 左↔中、中↔右，画法同 1.3 | ✅ |
| 6.3 | 左栏 gutter 靠中间一侧有 `>>`（应用到结果）和 `×`（忽略）；右栏对应 `<<` 和 `×` | ✅ |
| 6.4 | 冲突用红色；只有一方改动的块按类型着色；已解决的块变淡或消失 | ✅ |
| 6.5 | 冲突两边都应用时，第二个箭头变成 Append（追加在第一个后面） | ✅ |
| 6.6 | 结果栏 gutter 显示魔棒，可自动解决简单冲突；工具栏有「Resolve simple conflicts」 | ✅ |
| 6.7 | 工具栏：↑↓ 跳到上一个/下一个变更，Apply All Non-Conflicting Changes（左 / 全部 / 右三个按钮），空白、高亮、折叠、同步滚动、齿轮、帮助，右侧显示「N changes, M conflicts」 | ✅ |
| 6.8 | 底部按钮：Accept Left、Accept Right、Cancel、Apply；还有冲突未解决时点 Apply 会提示确认 | ✅ |
| 6.9 | 结果栏可自由编辑，编辑后重新计算各块状态 | ✅ |
| 6.10 | 可以切换显示 Base（三栏变成和 Base 对比） | ❌ |

## 7. 实现方案

AS 的做法是「几个独立编辑器 + 中间画连接块 + 同步滚动」，我们照这个结构做，不再用对齐表：

1. **`DiffPane`（新的文本面板）**：一个栏就是一个 `DiffPane`。按行渲染（固定行高，虚拟列表），支持行底色、单词高亮、语法高亮、折叠占位行、gutter（行号 + 按钮 + 复选框）放左边或右边、error stripe。先做只读（可选择复制），再加编辑（光标、选择、输入法、撤销），这样可编辑栏也完全受我们控制，滚动位置可以精确同步。
2. **`Divider`（连接块）**：用 GPUI 的 `canvas` + `PathBuilder`，根据两栏当前的滚动位置画梯形，和 AS 的 `DiffDividerDrawUtil` 一样。
3. **`SyncScroll`**：用变更列表把一栏的行号映射到另一栏（变更外按偏移量平移，变更内按比例），滚动一栏时设置另一栏的偏移。
4. **双栏 diff** = 2 个 `DiffPane` + 1 个 `Divider`；**Merge** = 3 个 `DiffPane` + 2 个 `Divider`；**单栏** = 1 个 `DiffPane`（删除行作为只读块插入）。
5. 现有的 diff 计算（`git/diff.rs`）、合并块计算（`git/merge.rs`）、部分提交逻辑都保留，改成输出「每栏的行 + 变更范围列表」。

## 8. 分阶段

1. ✅ **双栏 diff 结构**：两栏独立、gutter 镜像、分隔条连接块、同步滚动、`>>` 和复选框放进 gutter、标题条、error stripe、完整工具栏和统计。
2. ✅ **可编辑**：`DiffPane` 支持编辑，右栏工作区文件可直接改，实时重算并保存；语法高亮。
3. ✅ **三栏 Merge**：用同一套部件重写，`>> × <<`、冲突配色、魔棒、Apply Non-Conflicting 三个按钮、结果栏直接编辑。
4. **补齐细节**：Ctrl 切 Append、行级部分提交、右键菜单、齿轮菜单各项、空白/高亮的其余选项、跨文件导航、单栏可编辑。
