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
| 左/右/下 工具窗口条 | Commit、Git（Log/Console）、可拖拽、可隐藏、记住尺寸 | M1 | ✅（Project、Commit、Pull Requests、Changes、Find、Terminal、Git、Notifications；按钮可拖到左 / 右 / 下或在右键菜单 Move to，Hide；每侧同时开一个；Hide All（Ctrl+Shift+F12）、Hide Active（Shift+Esc）；布局和三侧尺寸写入设置） |
| Find 工具窗口 | 与 Git 同级的独立工具窗口（`Alt+3`） | M9 | ✅（不再是 Git 窗口里的标签；像 AS 一样第一次搜索后才出现在侧边栏，默认在下方、Terminal 和 Git 上面；关掉最后一个结果标签或点 Hide 收起；可拖到左/右侧） |
| Terminal 工具窗口 | `Alt+F12`，侧边栏按钮，在项目目录打开终端 | M9 | ✅（alacritty_terminal 做终端仿真，Windows 用 ConPTY；Windows 默认 PowerShell，Linux/macOS 用 $SHELL（macOS 为登录 shell）；"Local"、"Local (2)"… 多标签，+ 新建（Ctrl+Shift+T），⌄ 选其他 shell（Windows：Windows PowerShell / Command Prompt / PowerShell 7 / Git Bash / WSL，按已安装的列出；Linux/macOS：/etc/shells），⋮ 里 New Tab / Close Tab / Clear Buffer；shell 退出时关标签；256 色和真彩色、粗体/下划线、中文宽字符、输入法；鼠标拖选、双击选词、三击选行，Ctrl+C 有选区时复制否则发中断，Ctrl+V / Shift+Insert 粘贴（支持 bracketed paste），右键 Copy / Paste / Select All / Clear Buffer；滚轮看历史（全屏程序如 less/vim 里滚动发方向键）；Ctrl+E、Ctrl+N 等交给 shell，Alt+F12、Shift+Esc、Alt+数字、Ctrl+Shift+A 仍归 IDE。Linux 实测，Windows 需在真机验证） |
| Device Manager 工具窗口 | 右侧栏按钮（Notifications 下面），像 AS 一样管理安卓设备和模拟器 | M10 | ✅（SDK 按 AS 的顺序找：项目 local.properties 的 sdk.dir、ANDROID_HOME、ANDROID_SDK_ROOT、默认安装目录（Windows `%LOCALAPPDATA%\Android\Sdk`、macOS `~/Library/Android/sdk`、Linux `~/Android/Sdk`）、PATH 上的 adb；AVD 从 ANDROID_AVD_HOME 或 `~/.android/avd` 读。表格列：状态（在线绿点、启动中转圈、未授权警告）、Name（设备图标，第二行 `Android 14.0 ("UpsideDownCake") \| x86_64` 或 Starting up / Shutting down / Booting / 错误）、API、Type（Virtual / Physical）；点列头排序（默认 Name 升序），分组按钮 Group By None / Form Factor / Status / Type，组可折叠；虚拟设备 ▶ 启动（Quick Boot）/ ■ 停止（`adb emu kill`），启动失败弹通知并带模拟器日志里的原因；⋮ 菜单和右键：Cold Boot、Wipe Data、Delete（确认，运行中先停）、View Details（Summary：API、分辨率 px/dp、密度、ABI、存储、电源、磁盘占用；AVD 的 config.ini 属性；复制到剪贴板）、Show on Disk；真机只有 ⋮；空列表 "No devices connected." + "Add a new device..."；只在窗口打开时每 2 秒刷新 adb。Wi‑Fi 配对（工具栏 Wi‑Fi 按钮或 + 菜单）：AS 同款对话框 "Pair devices over Wi-Fi"，"Pair using QR code" 标签显示 `WIFI:T:ADB;S:studio-…;P:…;;` 二维码，设备扫码后自动 `adb pair`、等它连上（最多 2 分钟）并提示 "<设备> connected over Wi-Fi"，"Scan new device" 重新生成；"Pair using pairing code" 标签列出等待配对的设备，Pair 打开 6 格配对码输入（逐格跳、可整段粘贴），Pairing with device… → Connecting… → connected / 失败提示；adb 太旧或不支持 mDNS 时显示 AS 的错误文案。真机行有 Start Mirroring / Stop Mirroring 按钮（见 Running Devices）。开着无线调试、但 adb 没有通过 Wi‑Fi 连上的手机（`adb mdns services` 里的 `_adb-tls-connect` 服务）像 AS 一样单独列一行，第二行 "Available for Wi-Fi pairing"，名称和 API 取自之前连过时记下的信息（~/.cache/Junction/known-devices.json，没连过就显示序列号），行上 Wi‑Fi 按钮（⋮ 里也有 Pair device over Wi-Fi）先 `adb connect`，连上提示 "<设备> connected over Wi-Fi"，没配对过则打开配对对话框；连过的手机（known-devices.json 里有）出现无线调试服务时自动 `adb connect`（同一地址 30 秒最多一次），不用手动点；关掉再打开无线调试后端口会变，同一次刷新里连上新端口、断开旧地址，同一行原地从 Offline 变成在线（Wi‑Fi 连接的行按设备硬件序列号标识，换端口不换行；连不上时这一行改为 "Available for Wi-Fi pairing"），不再出现两条。API 36 起 API 列带小版本（36.0、37.0），API 37 显示 Android 17.0 ("CinnamonBun")。不做：+ 新建虚拟设备向导、Firebase 云端设备。用模拟的 adb / 模拟器在 Linux 实测，Windows 需真机验证） |
| Running Devices 工具窗口（设备镜像） | 右侧栏按钮（Device Manager 下面），像 AS 一样实时显示并操作设备屏幕 | M10 | ✅（通过 adb 推送 scrcpy-server 3.3.3（Apache 2.0，assets/scrcpy）到设备，adb forward 取 H.264 视频流，OpenH264 解码。Device Manager 真机行点 Start Mirroring 打开标签（再点 Stop Mirroring 关闭）；标签名 "<设备> API <n>"，绿点表示在线，× 停止；+ 菜单：Connected Devices（未镜像的已连接设备）、Virtual Devices（启动 AVD，开机后自动镜像）、Pair Devices Using Wi-Fi；空状态显示 AS 文案和 Device Manager 链接；⋮：Show Zoom Controls、Close Tab。工具栏：Power、Volume Up/Down、Rotate Left/Right（关自动旋转并设 user_rotation）、Back/Home/Overview、Hardware Input（开启后所有按键包括 IDE 快捷键都发到设备）、Direct Text Input（铅笔图标，默认开，关掉即结束设备上的写字小程序、释放 UI 自动化连接给 UI 测试用，再点打开）、Take Screenshot（对话框预览，Recapture / Copy to Clipboard / Save）、Record Screen（设备上 screenrecord，按钮显示计时，再点停止并保存 mp4）。屏幕：单击/拖动为触摸，滚轮滚动，键盘输入（ASCII 按字符，中文等非 ASCII 由推到设备上的小程序（assets/text-input，源码 tools/text-input，以 shell 身份通过 UiAutomation 设置焦点输入框的文字和光标）直接写进输入框，不经过输入法和剪贴板，不弹"已粘贴"提示和剪贴板浮窗；写不进去时（比如没有焦点输入框）退回 AS 的 Gboard Unicode 组合输入（Ctrl+Shift+U、十六进制码、Enter，需默认输入法是较新版 Gboard），都不行就丢弃并通知原因，不再用剪贴板粘贴；小程序在镜像连上时就后台启动（ASCII 不等它），启动成功、被占用（列出占用的 UI 自动化程序，如 Maestro）或失败都弹通知（同样结果不重复弹），失败后打字时每 30 秒后台重试，启动时等手机释放旧的 UI 自动化连接（最多约 5 秒），失败时通知里列出手机上占用的进程，启动后跟镜像会话同生命周期；连续输入来不及发的文字合并成一段一次写入，不再逐字出现，Enter/Backspace/方向键/Ctrl 组合按键码），输入法组合输入；剪贴板双向同步（设备复制自动到电脑，电脑剪贴板变化自动到设备，最长 5000 字符）；右侧浮动缩放按钮（放大、缩小、1:1、适应窗口），缩放后右下角短暂显示百分比；右键菜单 Take Screenshot / Record Screen；收起 Running Devices 时保持连接（不绘制画面），再打开立即显示，收起超过 10 分钟才结束设备上的 scrcpy-server，之后再打开自动重连；关标签、Stop Mirroring、退出程序都会结束它；断开时显示原因和 Reconnect，设备从 adb 消失则关闭标签。不做：Backup/Restore App Data、设备外框、多屏、音频转发、Device UI Shortcuts。用模拟的 adb 和设备端在 Linux 实测，Windows 需真机验证） |
| Device Explorer 工具窗口 | 右侧栏按钮（Running Devices 下面），像 AS 一样浏览设备文件和可调试进程 | M10 | ✅（Device Manager 在线设备行 ⋮ 菜单 Open in Device Explorer 直接打开并选中该设备；顶部设备下拉（名称 + 灰色 `Android 13.0 ("Tiramisu")`），没有设备时显示 AS 文案。Files 标签：`adb shell ls -al` 列目录，树表格列 Name（文件夹/文件类型图标，符号链接带箭头角标，链接到目录的可展开，悬停显示 Link target）、Permissions、Date（yyyy-MM-dd HH:mm）、Size（B/KB/MB，同 AS 算法）；有 root（adb root 或 su 0）时直接访问，否则像 AS 一样 /data 下显示 app、data、local，/data/data 下按 `pm list packages` 列出应用，进入应用目录用 `run-as`；工具栏 New Directory、Save As、Upload、Delete、Synchronize、包过滤（只看项目 applicationId 的 /data/data 目录，项目里找不到 applicationId 时禁用）；右键单选：New › File / Directory、Open、Save As… Ctrl+Shift+S、Upload… Ctrl+Shift+O、Delete… Delete、Synchronize Ctrl+F5、Copy Path Ctrl+Shift+C，多选：New 置灰、Save To…、Delete…、Synchronize、Copy Paths；New 对话框（默认 NewTextFile.txt / NewFolder，名字含 / 或为空时提示并禁用 OK，失败弹错误后重新输入）；Delete 确认文案同 AS，失败列出出错项；Save As 单文件选保存路径，目录或多选选目标文件夹，`adb pull`（run-as 文件用 `exec-out run-as cat`），完成后通知 "Successfully downloaded N files … in …"；Upload 选本机文件（没有系统文件对话框时改为输入路径），也可以把文件拖到目录上，`adb push`（run-as 目录先推到 /data/local/tmp 再 cp）；双击文件或 Enter 拉到 `~/.cache/Junction/device-explorer/<设备>/` 并在编辑器只读打开；方向键/Shift/Ctrl 多选、左右键展开收起；窄窗口时表格横向滚动。Processes 标签：通过 adb server 的 track-jdwp 列出可调试进程，再对每个进程发 DDM HELO 取名称、包名、ABI、VM、User ID、是否 Waiting for debugger（不回应的进程用 ps 补名称）；列 Process Name、PID、ABI、VM、User ID、Debugger，按名称排序，每 2 秒刷新；工具栏和右键：Kill process（JDWP VM Exit）、Force stop process（am force-stop）、Clear app data（pm clear）、Uninstall app、Refresh、包过滤；Ctrl+C 复制 "名称: PID"；没有进程时显示 "No debuggable process on device"。不做：Attach debugger（没有调试器）、Backup/Restore app data、传输进度条和取消。用模拟的 adb（含 JDWP 服务端）在 Linux 实测，Windows 需真机验证） |
| 状态栏 | 分支、行分隔符、后台任务进度、通知 | M1 | ✅（面包屑 项目 › 路径、后台任务转圈、索引状态、行:列（点开 Go to Line）、LF/CRLF、UTF-8、缩进、只读锁、当前分支（点开分支弹窗）；通知进右侧 Notifications 工具窗口，未读时铃铛带点） |
| VCS 操作弹窗 | `Alt+\``（macOS `Ctrl+V`）快速操作列表 | M3 | ✅（数字键 1–9 快选、上下键、回车，显示各平台快捷键） |
| 对话框与弹窗 | 居中打开；Esc 关闭；拖动标题栏移动 | M9 | ✅（所有对话框、列表弹窗（Choose Declaration、Usages、Recent Files、File Structure…）拖标题栏移动，开着时位置保持；Find in Files / Search Everywhere 拖标题行空白处移动，位置记住；这两个弹窗焦点在结果列表或预览里时 Esc 也能关闭） |
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
| 跟踪分支合并标签（2026-10-21，照 IntelliJ 的 `GitRefManager.groupForTable`）：本地分支和它跟踪的远程分支（没设 upstream 时找同名的 `<remote>/<分支>`）在同一个提交上时合成一个标签 “origin & main”，图标叠两个标签（下面紫色只露左边，上面绿色）；当前分支的标签排第一，+N 按合并后的标签数算。详情面板仍逐个列出 | M1 | ✅ |
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
| 文件历史（Show History）：Log 标签页 + 该文件的 diff | M3 | ✅（按 IntelliJ File History 布局：Branch: HEAD 过滤、无分支图的 作者/日期/提交信息 表格、tag/分支图标、工具栏 Refresh / Show Diff / Show All Affected Files / 视图选项（Commit Timestamp、Show Columns、Show Details、Show Diff Preview、Diff Preview Location）/ Open on GitHub / Show in Git Log、右侧 diff 预览、AS 的右键菜单；--follow 跟踪重命名，History Up to Here） |
| 选中内容历史（Show History for Selection） | M4 | ✅（文件编辑器右键 Git → Show History for Selection，用 git log -L 打开 Log 标签页） |
| Annotate with Git Blame：在编辑器 gutter 里逐行显示日期/作者（View 子菜单可加提交哈希），按时间着色，悬浮详情，跟随未保存的编辑；右键 Annotate Revision / Annotate Previous Revision（新增该文件的提交上置灰）/ Show Diff / Select in Git Log | M3 | ✅ |
| Diff 里的 Annotate with Git Blame：右键行号 gutter 打开，左右两侧各自在行号旁显示 blame 列（左侧按旧版本、右侧按新版本或工作区当前文本，未提交行为 Not Committed Yet），悬停的提交在两侧同时高亮并显示提交详情，单击跳到 Git Log；右键 blame 列：Annotate Revision / Annotate Previous Revision / Show Diff / Select in Git Log / Copy Revision Number / View / Close Annotations；切换文件时保持开启，编辑后自动重新 blame；并排和统一视图都支持 | M3 | ✅ |
| Diff 行号 gutter 右键菜单（与 AS 一致）：Annotate with Git Blame、Show Diff in Separate Window（独立窗口打开同一 diff）、Show Whitespaces、Show Line Numbers、Show Indent Guides、Soft-Wrap、Highlighting Level（All Problems / Syntax / None）、Breadcrumbs（Top / Bottom / Don't Show，每侧显示光标处的声明链，XML 显示标签链，可点击跳转）、Align Changes In Side-by-Side Diff | M4 | ✅ |
| 文件查看器里的变更标记（gutter）：点击看 diff、回滚 hunk、stage hunk | M4 | ✅（行号旁的 gutter 竖条：新增绿、修改蓝、删除处灰色楔形；点击弹出 AS 式变更弹窗：上一处 / 下一处（循环）、Rollback（可撤销）、Show Diff、Copy、Stage（只把这块写入 index，其它已暂存内容保留），下方显示 HEAD 里的原内容；Esc 或点外面关闭，打字时自动关闭；Rollback Lines（Ctrl+Alt+Z）；未版本控制的文件像 AS 一样没有变更标记；右键行号 gutter 的菜单见下一行） |
| 编辑器行号 gutter 右键菜单（与 AS 一致）：Annotate with Git Blame / Close Annotations、Add / Remove Bookmark（F11）、Add Mnemonic Bookmark… / Edit Mnemonic…（Ctrl+F11）、Soft-Wrap、Configure Soft Wraps…、Appearance（Show Line Numbers / Gutter Icons / Indent Guides / Whitespaces）、Configure Gutter Icons…（Settings › Editor › Gutter Icons） | M4 | ✅ |
| 书签：F11 切换、Ctrl+F11 选助记符（1–9、0、A–Z，被占用的会移过来）、Shift+F11 书签列表、Ctrl+数字跳到对应助记符书签；gutter 显示书签图标或助记符方框；编辑时书签跟着所在行移动；跨会话保存在 settings 目录 bookmarks.json | M4 | ✅ |
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
| 多仓项目（AS 对齐，2026-10-11） | M5 | ✅ Directory Mappings 与 AS 一致（Directory/VCS 表、+/−、Enable automatic mapping detection：开启时嵌套仓库自动成为根，关闭时右下角提示 Unregistered VCS roots detected：Add Roots / Configure / Ignore）；映射的目录不是仓库时提示 VCS root configuration problems（Configure…）；Log Paths 筛选列出各根并带根颜色色块；Commit/Changes 多仓默认按仓库分组，仓库节点带同色色块和分支；打开非仓库文件夹即为多仓项目（并列仓库，首次打开自动注册）；嵌套根；Commit/Log 跨根；Push 对话框按根分组（各自目标分支、New 标记、跳过无提交的根）；Fetch/Update 所有根，单根失败不影响其他根；冲突在所在根打开合并工具和横幅；Merge/Rebase/Pull/Stash/Tag/Reset 对话框 Git Root 选择；Stash/Shelf 列出所有根；Create/Apply Patch 跨根（项目相对路径）；编辑器/项目窗口 Git 操作走文件所在根，当前仓库跟随所选文件；分支弹窗同步分支控制（Checkout&Rebase/Merge/Rebase/Pull/Update/Rename/Delete）。分支弹窗 Common Local/Remote Branches；同步 Checkout 部分失败可 Rollback；跨仓 Undo Commit；多仓 Apply Patch 也把改动放进所选 changelist；外层仓库忽略掉的嵌套根目录在 Project 窗口只显示该根自己的文件（2026-10-21）。详见 reports/多仓对比与测试-2026-10-11.md |
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
| Ctrl+点击弹窗位置 | M9 | ✅（同 AS Windows：Ctrl+点击有多个目标时 Choose Declaration 在鼠标处弹出；点在声明本身上时在鼠标处弹出 Usages 列表（只有一处直接跳）；Ctrl+B 的弹窗出现在光标下方；Ctrl 悬停声明名也有下划线） |
| Ctrl+点击覆盖所有标识符 | M9 | ✅（对齐 AS：函数参数、局部变量、lambda/for/catch 变量按作用域解析，跳到声明；在这些声明上 Ctrl+点击列出作用域内用法；Kotlin 主构造器 `val`/`var` 参数作为属性索引，`obj.x` 可跳；类名/类型注解/返回类型/父类型跳到类定义；成员调用按接收者类型优先；C 系原型只在没有定义时才算声明。覆盖 Kotlin/Java/C/C++/Rust/Swift/Dart/Go/TS/JS/Python 等。`this`/`self`/`Self`（含 `this@Label`、Kotlin 扩展函数的接收者）跳到所在类型，`super`（含 `super<T>`）跳到父类；`this.x`/`super.x`/不带限定的成员调用沿继承层级找最近的声明；Kotlin `it` 和 Swift `$0` 跳到 lambda 的 `{`，Kotlin 访问器里的 `field` 跳到属性；命名参数（Kotlin/Python `x = 1`、Swift/Dart `x: 1`）跳到函数或构造器的参数） |
| 实现标记弹窗与库源码导航 | M9 | ✅（对齐 AS：gutter 实现/覆盖标记的弹窗标题带签名和数量（Kotlin 为 "Choose Implementation of onCreate(Bundle?) (6 found)"），居中加粗；每行类图标 + 可见性图标 + 类名（包名），右侧模块名（如 My_TV.app.main，取 settings.gradle 的 rootProject.name + 模块 + source set）或库名（Gradle: …@aar (classes.jar)）；项目在前、库在后；标题栏右侧 Open in Find Tool Window 把列表放到 Find 工具窗口。库里的子类、实现也进入标记和列表；在库源码里 Ctrl 点击声明、Find Usages 会同时搜项目和该库） |
| 编辑器 gutter 实现/覆盖图标 | M9 | ✅（行号与折叠图标之间，IntelliJ 官方图标：接口/抽象成员有实现时绿色 I↓，类有子类、方法被覆盖时蓝色 O↓，实现接口成员绿色 I↑，覆盖父类成员蓝色 O↑；点击一个目标直接跳，多个在鼠标处弹出 Choose Implementation / Subclass / Super Method 列表；继承关系来自索引：Java/Kotlin/Swift/Dart/TS/JS/Python/C++/ObjC 的父类型列表、Rust `impl Trait for T`，可传递；Java/Kotlin 里有同名类型时按文件的 import、包、`.*` 导入和 `Job.Factory` 这种带外层类名的写法认出指的是哪个，其他语言按简单名匹配；编辑后自动刷新） |
| Quick Documentation（悬停） | M7 | ✅（LSP hover，否则显示声明行与位置） |
| Find Usages（Alt+F7） | M7 | ✅（Find 工具窗口，按类别 › 文件 › 行分组；索引 + git grep，注释和字符串字面量里的文本不算（Kotlin / Dart 的 `$x`、Swift 的 `\(x)` 插值算代码），LSP references 标记“verified”；桥接两侧都列出；结果行高亮命中词，同名文件显示目录，右侧 Preview Source 预览选中结果；编辑器里也可 Ctrl+Alt+↓ / ↑ 跳到下一处 / 上一处） |
| Go to File / Class / Symbol（Ctrl+Shift+N / Ctrl+N / Ctrl+Alt+Shift+N） | M7 | ✅（打开 Search Everywhere 对应标签，见第 12 节） |
| Navigate Back / Forward（Ctrl+Alt+← / →） | M7 | ✅ |
| Project 工具窗口（Alt+1）、Select in Project View（Alt+F1） | M7 | ✅ |
| Project 窗口视图切换：Project（根节点带绝对路径）/ Project Files / Production / Tests / Open Files / Changed Files | M8 | ✅（Open Files 即当前打开的标签） |
| Project 窗口头部：New（File / Directory）、Select Opened File（准星）、Expand All、Collapse All | M8 | ✅ |
| Project 窗口选择与展开（与 AS 一致） | M9 | ✅（单击只选中，双击、点箭头、→ / ← 或 Enter 才展开/折叠；Ctrl·Cmd+单击增减、Shift+单击按范围选、Shift+↑↓/Home/End/PgUp/PgDn 扩展选区、Ctrl+A 全选；多选时右键菜单的 Cut / Copy / Copy Path / Delete / Git / Create Gist 作用于整个选区，Rename 置灰，选中两个文件时 Compare With…（Ctrl+D）直接比较这两个；把选中项拖到文件夹上弹出 Move（按住 Ctrl 为 Copy）。Commit、Changes、Log 的改动文件与分支树、Shelf、Stash、Find 结果、Repository at … 树同样改为单击选中、双击或点箭头展开） |
| Project 窗口右键里原来置灰的项（2026-10-21）：Find Usages（Alt+F7，Java / Kotlin 文件：查文件里同名类的用法）；Refactor › Move…（F6）/ Copy…（F5）对话框（To directory 相对工程，不存在就建；复制一项时可改 New name；不覆盖已有文件）；Reformat Code（Ctrl+Alt+L）/ Optimize Imports（Ctrl+Alt+O）对文件直接做，对目录先确认，处理其中所有 Java / Kotlin 文件，打开着的文件改在编辑器里（可撤销），其余写盘，右下角提示改了几个文件；Open Module Settings（F4，工程根目录和含 build.gradle 的模块目录）打开 Project Structure；Local History › Show History… / Put Label…；Mark Directory As › Excluded / Cancel Exclusion（排除的目录存在 .git/junction/roots.excluded，Project 窗口显示排除图标，索引、Go to File、Find in Files 都跳过；四种 Root 由 Gradle 决定，置灰）；Repair IDE on File（重新索引选中的文件并按磁盘重载）；Override File Type（文件右键和编辑器标签右键：弹出 “Override File Type for 'x'” 类型列表，选中即按该语言高亮，打开着的编辑器立即切换；按文件绝对路径存在配置目录 file-type-overrides，跨工程有效，文件移动或改名时跟着走；已覆盖的文件改显示 Revert File Type Override）；Analyze › Inspect Code…（先选范围：Whole project / 所在 Gradle 模块 / 当前文件、目录或选中的文件；后台跑语法错误和 Java / Kotlin 未用 import 两项检查，结果进 Problems 窗口新的 “Inspection of …” 标签，双击跳到源码；没问题时提示 “No suspicious code found”；范围里有 Android 模块时再按 Build Variants 选的变体跑 Gradle 的 `lint<Variant>`（多个模块一次 `--continue` 跑完），读 `build/reports/lint-results-<variant>.xml`，问题以 “消息 [IssueId]” 并进同一个标签，结束时提示找到几条；Lint 因错误让任务失败时照样读报告。Lint 部分只用模拟的 gradlew 验证过，需在有 Android SDK 的环境实测）、Code Cleanup…（同样选范围，删掉未用的 import）、Analyze Dependencies…（同样选范围，打开 “Dependencies of …” 依赖查看器：左边范围内的 Java / Kotlin 文件，右边选中文件用到的工程文件（按 import 和包解析，同包同目录的名字也算，点一下打开）和库的包；Signal 的 app 模块 4235 个文件约 0.4 秒） | M9 | ✅ |
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
| 右键（与 AS 一致）：Close（Ctrl+F4）/ Close Other Tabs / Close All Tabs / Close Unmodified Tabs / Close Tabs to the Left / Right / Close All Read-Only（有固定标签时多 Close All but Pinned）、Copy Path/Reference…、Split Right / Split and Move Right / Split Down / Split and Move Down、Pin Tab、Open Tab in New Window（Shift+F4，标签移到独立窗口）、Configure Editor Tabs…（Settings › Editor › Editor Tabs：关闭按钮、Mark modified (*)、预览标签、标签上限）、Bookmarks（文件书签）、Override File Type（未实现，置灰）、Open In、Local History（Show History… / Show Project History… / Recent Changes / Put Label…）、Git、Rename File…、Create Gist… | M8 | ✅ |
| 标签悬停显示文件完整路径；修改未保存的文件关闭按钮位置显示圆点，悬停变 × | M8 | ✅ |
| Copy Path/Reference… 弹窗（标签、Project 视图、编辑器 Copy / Paste Special）：Absolute Path（Ctrl+Shift+C）、File Name、Path from Content Root、Path from Source Root、Path From Repository Root、GitHub / GitLab Repository URL（跟踪的远程分支，否则当前提交；编辑器里带选中行），每行右侧预览要复制的内容，↑↓ Enter 或点击复制 | M8 | ✅（内容根/源码根按 AS 的模块规则推断：Gradle source set 为 app/src/main，源码根为其下的 java、kotlin、res 等；Cargo、Dart、Swift Package、Go 模块也有对应规则） |
| 固定标签排在最前并显示图钉；拖动标签调整顺序 | M8 | ✅ |
| 中键关闭；Ctrl+F4 关闭；Alt+← / Alt+→ 切换（macOS 为 Cmd+Shift+[ / ]） | M8 | ✅ |
| 分屏：Split Right / Down、Split and Move Right / Down、Open in / Move to Opposite Group、Unsplit；标签可拖到另一组；某组标签关完后自动合并 | M8 | ✅（两组；同一文件在两组共用一份文本（2026-10-21）：一边的输入、撤销、保存立刻同步到另一边，另一边的光标留在原处，修改标记一起变；分屏时未保存的修改带过去。撤销历史各自一份） |
| 预览标签（Project 窗口 ⋮ › Behavior › Enable Preview Tab） | M8 | ✅（单击文件在一个斜体标签里预览，下一次预览替换它；编辑、双击文件或双击标签后变成普通标签） |
| 多行标签、双击最大化编辑器（2026-10-21） | — | ✅（Settings › Editor › Editor Tabs 的 Multiple rows：标签换行、标签栏变高，默认仍是一行可滚动；双击标签按 IntelliJ 的 `doProcessDoubleClick`：预览标签变成普通标签，否则切换 Hide All Tool Windows，再双击恢复） |

## 14. 编辑器代码分析（2026-10-14）

不常驻 Kotlin 语言服务：补全来自自有索引，报错来自 tree-sitter 语法检查和保存后的真实编译。代码按 Clean 架构分层（`src/domain`、`src/usecase`、`src/infra`、`src/ui`）。

| 功能 | 状态 |
|---|---|
| 自动弹出补全：输入标识符或 `.` 后弹出；Ctrl+Space 手动补全 | ✅（候选：局部变量和参数、当前类成员、文件内声明、项目和外部库的类/函数/常量、关键字；IntelliJ 驼峰匹配（`gTV` → `getTextView`），同等匹配下按 局部 > 成员 > 本文件 > 项目 > 库 排序） |
| `x.` 后列出接收者类型及其父类型的成员 | ✅（类型来自声明 `val x: T` / `T x` / `x = T(`、`this.`、`super.`；推断不出类型时，输入字母后按名字列出所有成员） |
| 自动 import（Kotlin / Java）：选中的类或 Kotlin 顶层函数未导入时，按字母顺序插入 import | ✅（已导入、同包、`java.lang` / `kotlin.*` 默认包、同名类已导入时不加） |
| 函数补全插入 `name()`，光标放在括号内；后面已有 `(` 时只补名字 | ✅（索引没有参数信息，无参函数光标也在括号内） |
| 语法错误红色波浪线，悬停显示原因（`'}' expected` 等），输入停顿 0.3 秒后更新 | ✅（tree-sitter 的 ERROR / MISSING 节点；类型错误等语义错误由下一行的编译给出） |
| 保存 Kotlin / Java 文件后，后台编译所在 Gradle 模块，编译器的错误和警告标到编辑器（悬停看信息），修好再保存后消失 | ✅（Android 模块 `:m:compileDebugJavaWithJavac`，JVM 模块 `:m:classes`；用项目的 gradlew；状态栏显示 Compiling…；同一模块编译中再次保存会排队一次；Gradle 守护进程闲置 15 分钟退出；编译失败但没有编译器信息时（构建脚本错、没有 JDK）弹通知显示 Gradle 输出末尾。没有 JAVA_HOME 且 PATH 里没有 java 时，使用 Android Studio 自带的 jbr） |
| Settings › Build, Execution, Deployment › Compiler › Compile on save | ✅（默认开） |
| 快速修复（Alt+Enter）、Reformat、参数提示 | ✅（见第 33–35 节；Problems 工具窗口见第 18 节） |

Linux 实测：mytv-android（补全、自动 import、语法错误），最小 Kotlin/JVM Gradle 项目（保存后编译：冷启动约 40 秒，热编译约 1.5 秒）。Windows / macOS 需真机验证。

## 15. Gradle Sync（2026-10-14）

Android Studio 的 Sync Project with Gradle Files，轻量版：只让 Gradle 解析各模块的编译 classpath（缺的依赖和源码包顺带下载），外部库按解析结果精确列出并只重新索引变了的库；不重建项目模型，打开项目不自动同步。代码：`src/domain/sync.rs`、`src/usecase/sync_project.rs`、`src/infra/gradle_sync.rs`、`src/ui/sync_service.rs`。

| 功能 | 状态 |
|---|---|
| Sync Project with Gradle Files：File 菜单、标题栏 Gradle 按钮（仅 Gradle 项目显示）、Ctrl+Shift+O（macOS ⇧⌘O） | ✅ |
| 打开项目时：build 文件和上次同步时一样则什么都不做；变了（或从未同步过）弹 “Gradle files have changed since last project sync…” 通知，带 Sync Now / Ignore these changes | ✅ |
| 项目打开期间 build 文件变了（切分支、pull、外部编辑后刷新）自动同步；同步失败后文件不再变化时不自动重试 | ✅（build 文件 = build.gradle(.kts)、settings.gradle(.kts)、gradle.properties、*.versions.toml、gradle-wrapper.properties） |
| 解析范围：每个模块的 debug 变体 Compile / UnitTest / AndroidTest classpath，JVM 模块 compileClasspath / testCompileClasspath；同时请求源码包 | ✅（init script 加 `junctionSync` 任务，关闭 configuration cache 运行；included build 跟主 build 一起） |
| 同步成功后 External Libraries 按解析结果精确列出（含传递依赖的确切版本），未同步过时仍用读 build 文件 + Gradle 缓存的推断 | ✅（记录在 `~/.cache/Junction/gradle-sync/`） |
| 解析不到的依赖弹警告通知 “Failed to resolve: g:a:v”；同步失败弹错误通知并显示 Gradle 输出，保留上次的库列表 | ✅ |
| 状态栏显示 Gradle sync running… | ✅ |
| Build 工具窗口的 Sync 标签页：Gradle 输出流式显示在右侧控制台，左侧树是「项目: finished at … 用时 — N errors, M warnings」，下面列每个没解析到的依赖（Failed to resolve）和失败原因；同步中显示 syncing… 并可 Stop；同步开始时自动切到 Sync 标签，失败时自动打开 Build 窗口 | ✅（2026-10-20） |
| Build Variants 选变体 | ✅（见第 16 节） |
| 模块 / 源集模型 | ❌ 未做 |

Linux 实测：最小 Kotlin/JVM 项目（okhttp、gson、一个不存在的依赖）：打开时提示、Sync Now 后库从 8 个变 10 个、切到加了 retrofit 的分支后自动同步变 11 个、build 脚本写坏时报错且库不变。热守护进程下同步 1–3 秒。Windows / macOS 和真实 Android 项目需真机验证。

## 16. Build + Run（2026-10-15）

Android Studio 的 Make Project、Run 'app'、Stop，以及 Build、Run、Build Variants 三个工具窗口。代码：`src/domain/run.rs`、`src/usecase/run_app.rs`、`src/infra/gradle.rs`（流式跑任务、读 `output-metadata.json`）、`src/infra/adb_device.rs`、`src/ui/run_service.rs`、`src/ui/run_views.rs`。

| 功能 | 状态 |
|---|---|
| 标题栏（Gradle 项目才显示）：Make Project 锤子、设备下拉（在线设备 + 可启动的模拟器）、运行配置下拉（每个 application 模块一个，名字同 AS）、Run（运行中变 Rerun）、Stop（只在有东西可停时出现）、Sync | ✅（选的设备记住；选了没开的模拟器会先启动并等它上线） |
| Make Project（Ctrl+F9 / ⌘F9）：所有模块按当前变体 `:m:assemble<Variant>`，JVM 模块 `:m:assemble` | ✅ |
| Run 'app'（Shift+F10 / ⌃R）：`:app:assemble<Variant>` → 读 `build/outputs/apk/**/output-metadata.json` 得 applicationId 和 APK → `adb install -r -t`（多 APK 用 install-multiple）→ `am force-stop` → `cmd package resolve-activity` 找启动 Activity（TV 应用找 LEANBACK_LAUNCHER）→ `am start` → `pidof` | ✅ |
| Run 窗口（Alt+4）：AS 的输出 “Launching 'app' on …”、“Install successfully finished in …”、`$ adb shell am start …`、“Connected to process … on device …”；安装失败显示 `INSTALL_FAILED_…` 原因；Rerun / Stop | ✅ |
| Stop（Ctrl+F2 / ⌘F2）：取消构建（Windows 下杀整个进程树），停掉应用（`am force-stop`） | ✅ |
| Build 窗口：首行 “Executing tasks: […]”，状态（运行中 / 成功 / 失败 / 取消、时长、错误和警告数），左侧问题列表点击跳到源码行列，右侧 Gradle 输出（虚拟列表、错误行标红、自动滚到底） | ✅（解析 Kotlin K1/K2、javac、AAPT 报错；构建的错误同时标到编辑器） |
| Build Variants 窗口：每个 Android 模块的 Active Build Variant 下拉，选了后重新 Sync；Run、Make、保存后编译、Sync 都用所选变体 | ✅（变体来自 Sync 记录；没 Sync 过时按 build 文件猜模块，变体为 debug / release） |
| 主菜单 Build（Make Project、Sync、Select Build Variant…）和 Run（Run 'app'、Stop）；View › Tool Windows 加 Build、Run、Build Variants；Find Action 可搜到 | ✅ |
| Run/Debug Configurations 对话框（2026-10-16，`src/ui/run_config_dialog.rs`）：标题栏配置下拉顶部和 Run 菜单的 “Edit Configurations…”；左边 Android App 组的配置列表，+ 新建（“Unnamed”）、− 删除、复制（“app (1)”）；右边 Name、Module（项目的 application 模块）、Installation Options 的 Deploy（Default APK / Nothing）、Launch Options 的 Launch（Default Activity / Specified Activity + Activity / Nothing）和 Launch Flags；名称为空或重名、Specified Activity 没填时提示且不能确定。OK 后存到 `run-configurations.json`（按构建根目录），标题栏下拉列出它们；保存过就不再按模块自动生成。Run / Debug 照配置：Deploy Nothing 不安装，Specified Activity 用 `pkg/.SettingsActivity`（以 `.` 开头补包名，含 `/` 原样），Launch Nothing 只构建安装，Launch Flags 追加到 `am start` 命令行（控制台显示的命令同步） | ✅ |
| Before launch 任务列表（2026-10-22，Edit Configurations 右侧最下面）：默认 “Gradle-aware Make”（模块的 assemble 任务）；+ 可加 Run Gradle task（输入任务名，如 `clean :app:lint`）和 Run External tool（输入程序和参数，双引号包住含空格的参数，在构建根目录运行）；− 删除、✎ 或双击修改、↑ ↓ 调整顺序。Run / Debug 按顺序执行，输出进 Build 窗口，任一项失败就不再安装；去掉 Make 时直接部署已有的 APK | ✅ |
| 多设备同时运行（2026-10-22）：设备下拉最下面的 “Select Multiple Devices…” 对话框勾选几台（含没开的模拟器，运行时先启动），下拉显示 “Multiple Devices (2)”；Run 只构建一次，再逐台安装、启动，Run 窗口首行 “Launching 'app' on Multiple Devices.”，每行前面标出设备名；Stop 停掉所有设备上的应用；Debug 只调试其中第一台；在下拉里点单台设备即退出多设备模式 | ✅（用假 adb 的两台设备验证） |
| Gradle 运行配置（2026-10-22）：Edit Configurations 的 + › Gradle，填 Name、Gradle project（工程里的 Gradle 构建）和 Run（任务和参数，如 `test --tests com.example.CalcTest`、`clean :app:lint --info`），左边列表分 Android App / Gradle 两组，标题栏下拉带 Gradle 图标；Run 在 Run 窗口执行，首尾是 IntelliJ 的 “Executing '…'…” 和 “Execution finished '…'.”，Stop 可取消，编译错误同样标到编辑器；默认没有 Before launch 任务（可加）；Debug 不支持 | ✅ |
| JUnit 运行配置（2026-10-22）：Edit Configurations 的 + › JUnit，填 Name、Module（任一模块）、Test kind（All in package / Class / Method）和对应的 Package、Class、Method；类或方法没填时提示 IntelliJ 的 “No test class specified” / “Method name not specified”；左边列表多一个 JUnit 组，标题栏下拉带测试图标。Run、Debug、Run with Coverage 都走与 gutter 相同的 Gradle 单元测试流程（结果树、调试器连到测试 JVM、覆盖率），不进临时配置列表 | ✅ |
| 临时配置存为正式配置（2026-10-22）：选中从 gutter / Project 窗口跑出来的测试临时配置时，下拉里多一项 “Save 'CalcTest' Configuration”，存成 JUnit 配置（包、类或方法照原样）并选中，临时列表里去掉；构建原来用默认配置时默认的 app 配置一并保存；Rerun Failed Tests 那种多个过滤条件的不能存 | ✅ |
| Android Instrumented Tests 运行配置（2026-10-22）：Edit Configurations 的 + › Android Instrumented Tests，Module 只列 Android 模块（默认第一个有 `src/androidTest` 的），Test kind 为 All in Module / All in Package / Class / Method，包名没填时提示 “Package name not specified”；左边列表多一个 Android Instrumented Tests 组。Run 和 Run with Coverage 走与 gutter 相同的 `connected…AndroidTest` 流程（`class` / `package` 作为 instrumentation runner 参数，在所有连接的设备上跑），从 gutter 跑出的仪器测试临时配置也能 Save 成这种配置 | ✅（容器里没有 AGP 工程，只验证到 Gradle 收到 `:app:connectedDebugAndroidTest`） |
| Android Instrumented Tests 配置的 Debug、Deployment Target Options、Instrumentation 参数 | ❌ 未做（Debug 会提示不支持；Apply Changes 见第 60 节）|

Linux 实测（容器无法下载 Android SDK，用假 adb 和一个模拟 assembleDebug 的 Gradle 项目）：Run 走完构建、安装、启动、找到进程；Stop 发出 force-stop；Make Project 把 Kotlin 类型错误列在 Build 窗口，点击跳到 Util.kt 2:18 并在编辑器标红。真实 AGP 项目和真机需要在 Windows / macOS 上验证。

## 17. Logcat（2026-10-15）

Android Studio 的 Logcat 工具窗口（Alt+6 / ⌘6，底部）。代码：`src/domain/logcat.rs`（解析 `threadtime`、过滤语法、堆栈帧）、`src/usecase/logcat.rs`（按进程表给每行配包名，插入 PROCESS STARTED / ENDED）、`src/infra/adb_logcat.rs`、`src/ui/logcat_view/`；过滤输入的补全和历史规则在 `src/domain/logcat_filter.rs`，持久化 `src/infra/logcat_filter_store.rs`，界面 `src/ui/logcat_view/filter.rs`。

| 功能 | 状态 |
|---|---|
| 设备下拉：在线设备，显示 “型号 Android 16, API 36”；默认跟随运行目标设备；设备断开保留日志并标 [OFFLINE]；没有设备时显示 No connected devices | ✅ |
| 读取：`adb logcat -v threadtime -v year`（Android 7 以下自动去掉 `-v year`）；首次打开窗口才开始读，窗口隐藏时继续读；缓冲区同 AS 默认 1024 KB，超出丢最旧的 | ✅ |
| 包名：`ps -A -o PID,NAME`（老设备用 `ps`）对应 pid，遇到新 pid 或 ActivityManager 报进程死亡时刷新；未知进程显示 `pid-N` | ✅ |
| AS 的 `---- PROCESS STARTED (pid) for package x ----` / `PROCESS ENDED` 分隔行 | ✅ |
| 过滤（默认 `package:mine`）：`tag:` `package:` `process:` `message:` `line:` `level:` `age:`（s/m/h/d）`is:crash` `is:stacktrace`，`-key:` 取反，`key=:` 精确、`key~:` 正则，引号值；同键 OR、不同键 AND；不带键的词匹配整行；`package:mine` = 项目 applicationId + 最近一次 Run 的 applicationId | ✅（没检测到项目包名时显示 AS 的提示条） |
| 显示：Standard（日期时间、pid-tid、标签 23 列、包名 35 列、级别、消息）和 Compact（时间、级别、消息）；AS Default / Darcula 配色的级别徽标和消息颜色，标签按名字着色 | ✅ |
| 堆栈帧 `at a.b.C.m(File.kt:42)` 的 `File.kt:42` 是链接，点击按包路径找到源码（项目优先，其次外部库）并跳到该行 | ✅ |
| 左侧工具栏：Clear Logcat（同时 `logcat -c`）、Scroll to the End（滚到底自动跟随，往上滚停止跟随）、Soft-Wrap、Standard / Compact、Pause / Resume（显示 “Logcat is paused”）、Restart | ✅ |
| 选中行（Shift 扩选）Ctrl+C 复制 | ✅ |
| 过滤输入的补全：输入时弹出（Ctrl+Space 也可），键按 AS 的变体列出（`tag:` `-tag:` `tag~:` `-tag~:` `tag=:` `-tag=:`，`level:` `age:` `is:` 等），输入 `tag:` / `package:`（含 `mine`）/ `process:` 后列出日志里出现过的值，`level:` 列级别、`is:` 列 crash / stacktrace；前缀匹配优先，其次包含匹配，已输入部分加粗；弹窗在当前词下方，↑↓ 选、Enter / Tab 填入（填入键后接着弹值）、Esc 关闭、点击填入 | ✅ |
| 过滤历史和收藏：输入框左侧的历史按钮弹出 Favorites 和 History 两段，每条带匹配条数，点一条应用、× 删除；回车或离开输入框时记入历史（最近 20 条，同 AS）；右侧星标把当前过滤加入 / 移出收藏；跨会话保存在设置目录 `logcat-filters.json` | ✅ |
| 多个标签页（2026-10-16，`src/ui/logcat_view/tabs.rs`）：标题旁的标签条，+ 新建标签（命名照 AS 的 `UniqueNameGenerator`：“Logcat”、“Logcat (2)”…取第一个没用过的），每个标签各自的设备、过滤、暂停和日志，工具窗口隐藏时全部继续读；× 或中键关闭（只剩一个时不能关），关闭后停止读取；右键菜单 Rename Tab…（不能为空或与其他标签重名）、Close Tab、Close Other Tabs。实测（Xvfb，模拟 adb）：新建 “Logcat (2)” 设 `level:error` 只剩三条 E，切回 “Logcat” 仍是 `package:mine`；重命名为 “Errors”；关闭后 adb logcat 进程少一个 | ✅ |
| Screen Capture / Screen Record（2026-10-16）：左侧工具栏末尾 AS 的截屏和录屏按钮，对当前跟随的设备；截屏用 Running Devices 的截图对话框（Recapture、Copy to Clipboard、Save），录屏第二次点击停止并选择保存位置（`screenrecord`，同 Running Devices）。实测（假 adb 返回 PNG）：点击后弹出截图，Recapture 重新截取 | ✅ |
| 分屏（AS 的 `SplittingPanel` / Split Panels）：左侧工具栏最下面的 Split Panels 按钮，Split Right、Split Down 在旁边加一个面板，各自有设备、过滤和日志，可以继续分、拖动分隔线调大小；Close 关掉当前面板，旁边的顶上，标签页只剩一个面板时关整个标签页；面板矮时左侧工具栏可滚动 | ✅（2026-10-20） |
| 导出 / 导入日志文件（2026-10-21，`src/domain/logcat_file.rs`）：左侧工具栏 Export Logcat to File… 按 AS 的 `.logcat` 格式（JSON：metadata 里设备、过滤、项目 applicationId，logcatMessages 里每条的 header 和 message）写出，默认文件名 “设备名_日期.logcat”；Import Logcat from File… 读 `.logcat`、`adb logcat -v threadtime` 文本或 bugreport 的 logcat 段，在新标签页（以文件名命名）显示，恢复保存时的过滤（`package:mine` 换成文件里的 applicationId），标题显示 “文件名 (设备名)”；Go to File 打开 `.logcat` 也走这里。实测（Xvfb，假 adb）：`level:warn` 下导出，JSON 设备为 Google Pixel 9a；再导入，新标签页 4 条、过滤 `level:warn` | ✅ |
| Proguard / R8 反混淆（2026-10-21，`src/domain/retrace.rs`、`src/infra/r8_mapping.rs`）：左侧工具栏 Load Proguard Mappings… 给当前面板选 mapping 文件（没有文件对话框时用项目里最新的 `build/outputs/mapping/<variant>/mapping.txt`）；堆栈帧的 `r8-map-id-<id>` 自动找项目里 `# pg_map_id` 相同的 mapping（同 AS 的 `AutoProguardMessageRewriter`）。还原帧的类、方法、文件（mapping 的 sourceFile，否则按类名猜）和行号（行号区间换算），内联帧展开成多行接在下面，分不清的重载按 R8 写成 `<OR> at …`；`Caused by:` 等行的异常类名也还原。mapping 只建类索引，成员按需从磁盘读。实测（Xvfb，假 adb）：自动和手动两种方式都把 `at a.a.a(…:5)` 还原成 Util.check / MainActivity.crash / MainActivity.onClick 三行，链接可点 | ✅ |
| 过滤的 `name:` 键（2026-10-21，同 AS 的 `NameFilter`）：`name:"Slow calls"` 给过滤起名，不影响匹配；补全列表里有 `name:`；历史里显示粗体名字，悬停显示去掉名字的过滤，有同名的过滤时显示 “名字: 过滤”。实测（Xvfb）：三条带名字的过滤显示为 Mine、Errors: tag:OkHttp、Errors: level:error | ✅ |
| 标签页和分屏在重启后恢复（2026-10-21，`src/domain/logcat_layout.rs`）：按项目存进编辑器会话（`editor-sessions.json`），每个标签页的名字、分屏方向，每个面板的过滤、选中的设备、Standard / Compact、Soft-Wrap、Load Proguard Mappings 的文件、导入的日志文件（文件不在了就回到设备）；新建、关闭、重命名、分屏后两秒内保存，退出时也保存。实测（Xvfb）：两个标签页、第二个上下分屏，重启后标签名、分屏和三个过滤都恢复 | ✅ |

过滤补全实测（假 adb）：清空后输入 `ta` 弹出 `tag:` `tag~:` `tag=:`，回车后弹出 ActivityManager / MainActivity / OkHttp / SystemUi，接着输入 `ma` 剩 MainActivity（前缀）和 ActivityManager（包含），Tab 填入；输入 ` level:` 列出级别，点 info 后过滤为三条 ActivityManager 的 I 级日志；点星标收藏，再输入 `is:crash` 回车；历史按钮弹出 Favorites “tag:ActivityManager level:info (3)” 和 History “is:crash (0)”，点收藏项恢复该过滤。空输入框 Ctrl+Space 列出全部键，↓ 超出可见范围时列表跟着滚动。

Linux 实测（假 adb 模拟日志流和进程表）：打开窗口即显示 package:mine 的日志、启动和结束分隔行、崩溃堆栈；点 MainActivity.kt:7 打开文件并定位到第 7 行；level:warn 过滤、Compact、暂停提示条、Clear 发出 `logcat -c` 都正常。真机需在 Windows / macOS 上验证。截图：reports/Logcat-2026-10-15/。

## 18. Problems、TODO、Local History（2026-10-15）

代码：`src/ui/item_tree.rs`（按文件分组的树，Problems 和 TODO 共用）、`src/ui/problems_view.rs`、`src/domain/todo.rs`、`src/ui/todo_view.rs`、`src/domain/local_history.rs`、`src/infra/local_history_store.rs`、`src/usecase/local_history.rs`、`src/ui/local_history_window.rs`。

| 功能 | 状态 |
|---|---|
| Problems 工具窗口（底部）：File 标签页列出当前编辑器文件的问题（语法错误 + 保存后编译的错误和警告），Project Errors 标签页列出最近构建 / 编译报告的所有文件；文件节点显示目录和问题数，问题行显示图标、消息、行号 | ✅ |
| Problems 工具栏：上一个 / 下一个问题（打开并定位）、全部展开 / 折叠、Autoscroll to Source、选项（Show Warnings、Sort by Severity）；双击或 Enter 跳到源码，方向键移动 | ✅ |
| TODO 工具窗口（底部）：Project 和 Current File 标签页，找注释里的 `TODO` / `FIXME`（IntelliJ 默认模式，不区分大小写，只认注释），按文件分组，显示 “(行, 列) 文本” 并高亮 TODO 部分，顶部显示 “Found N TODO items in M files”；文件保存或变化后自动刷新 | ✅（模式不能自定义；没有按模块 / 包分组） |
| Local History：每次保存记录一个版本，文件第一次打开时记录原始内容；内容去重存放在 `~/.cache/Junction/local-history/<项目>/`；保留最近 5 个有改动的日子（IntelliJ 默认），超出的在打开项目后第一次保存时清理 | ✅ |
| Local History › Show History（File 菜单、编辑器右键、Find Action）：独立窗口，左边版本列表（时间到秒、标签），右边所选版本和当前文件的对比（当前文件可编辑）；Revert 把文件恢复到所选版本，打开的编辑器同步刷新 | ✅ |
| Local History 的目录和项目历史（2026-10-21）：Project 窗口右键目录 › Local History › Show History…、Show Project History…（也在 File › Local History）：左边是目录下各文件的版本（带文件名）和标签，中间列出所选时刻之后改过的文件，右边是所选文件当时与现在的对比；Revert 把这些文件一起恢复。Recent Changes（Alt+Shift+C）：最近 40 条记录（时间、文件、所在目录），点一条打开该文件的历史并选中那个版本，标签打开项目历史 | ✅ |
| Local History › Put Label…：给整个项目在此刻打标签，出现在每个文件的历史里 | ✅ |
| 外部修改自动记版本（2026-10-22）：Git 操作或其他程序改了有历史的文件，仓库刷新时记一个 “External change” 版本（只比较 mtime 变了的文件，后台单线程排队，和保存记录不会抢先后） | ✅ |
| Show History for Selection…（2026-10-22，编辑器右键 › Local History）：窗口标题 “Local History for Selection of X (lines a–b)”，只列出这几行有变化的版本，按 diff 把选区跟到旧版本里的对应行，右边是那几行当时与现在的对比；Revert Selection 只把这几行改回去 | ✅ |
| 历史窗口顶部搜索框（2026-10-22）：文件和选区历史 “Search by content”（按版本内容过滤），目录和项目历史 “Search by file name” | ✅ |

Linux 实测：TODO 窗口找到 3 条（含未跟踪文件），双击跳到对应行；Make Project 后 Problems 的 File 和 Project Errors 都列出 Util.kt 的类型错误，双击跳到 2:18；保存三次加一个标签后 Show History 列出全部版本，Revert 到最早版本后磁盘内容恢复。截图：reports/Problems-TODO-LocalHistory-2026-10-15/。

## 19. Android 资源（2026-10-15）

代码：`src/domain/android_resources.rs`（资源引用、values 解析、颜色解析）、`src/infra/resource_scanner.rs`（扫描各模块 `src/<sourceSet>/res`）、`src/usecase/android_resources.rs`（资源表、值解析、gutter 预览），接入 `src/index/service.rs` 和 `src/ui/file_editor/previews.rs`。

| 功能 | 状态 |
|---|---|
| 资源表：扫描所有模块所有 source set 的 `res/`，包括 values（string、color、dimen、bool、integer、style、array 等）、文件资源（drawable、mipmap、layout、menu、xml…）和布局里的 `@+id`；res 下文件保存或变化时增量更新 | ✅ |
| Ctrl+B / Ctrl+点击：Kotlin / Java 里的 `R.string.app_name`、XML 里的 `@string/app_name`、`@color/x`、`@drawable/x`、`@id/x` 跳到声明；多个限定符目录（values、values-zh…）都有时弹 Choose Declaration，按 AS 的目录顺序排列 | ✅ |
| 悬停：显示资源的值（跟随 `@color/a` → `@color/b` 引用链）和所在目录、文件 | ✅ |
| Find Usages（Alt+F7）：分 Declarations、代码里的引用、XML 里的引用三组 | ✅ |
| gutter 预览：代码和 XML 里引用颜色的行显示色块，引用 png / webp / jpg 图片的行显示缩略图（取最高密度目录的那张）；colors.xml 里每个颜色定义也显示色块 | ✅ |
| 点击 gutter 图标（2026-10-22）：XML 里颜色字面量（`#FF6200EE`、`"#0f0"`）的色块弹出取色器（调色板、HSLA 滑块、十六进制输入），选中后写回原处，保持原来的位数和大小写（不透明且原来没写 alpha 时写 `#RRGGBB`，否则 `#AARRGGBB`），可 Ctrl+Z 撤销；资源引用（`@color/x`、`R.color.x`、`@drawable/x` 等）的色块和缩略图跳到资源声明 | ✅ |
| Compose 颜色字面量（2026-10-22）：Kotlin 里 `Color(0xFF6200EE)`（也认 `Color(color = 0x…L)`）的行显示色块，按 Compose 的 ARGB 读（`0x6200EE` 没写 alpha 即透明）；点色块弹出同一个取色器，写回 `0xAARRGGBB`，保持原来的大小写 | ✅ |
| 框架资源（2026-10-22）：读 compileSdk 对应平台（没有就取最新装的）的 `platforms/android-N/data/res`，只读默认 `values` 和文件资源；Kotlin / Java 的 `android.R.string.ok`、XML 的 `@android:color/white`、`?android:attr/x`、`?android:x` 可 Ctrl+B 跳到 SDK 里的声明（只读打开）、悬停显示值，`android.R.color.x` / `@android:color/x` 的行显示色块 | ✅（用假 SDK 平台目录验证；真 SDK 的大小和内存未实测） |
| 字符串折叠显示值、框架资源的 Find Usages 和翻译、取色器的 “选资源” 页、Compose `Color(red, green, blue)` 分量写法 | ❌ 未做（Translations Editor 见第 31 节，vector drawable 见第 32 节，shape / selector 预览见第 39 节，Merged Manifest 见第 41 节；字符串折叠需要编辑器支持行内折叠，现在只有整行折叠） |

Linux 实测：MainActivity.kt 里 Ctrl+B `R.string.app_name` 弹出 values 和 values-zh 两个声明，悬停显示值；Ctrl+B `R.color.purple_500` 打开 colors.xml 第 2 行；Alt+F7 分组正确；MainActivity.kt 和 activity_main.xml 的 gutter 显示紫色、青色色块（`@color/accent` 跟随到 teal_200）和 ic_logo 缩略图。截图：reports/Android资源-2026-10-15/。

## 20. Gradle 文件辅助（2026-10-15）

代码：`src/domain/version_catalog.rs`（解析 `*.versions.toml`、访问器和目录内引用）、`src/domain/dependency_versions.rs`（找版本位置、Gradle 版本排序、选新版本）、`src/infra/catalog_files.rs`（找项目的 catalog，含 settings 里 `from(files(...))` 创建的）、`src/infra/maven_versions.rs`（Google Maven、Maven Central、Gradle Plugin Portal 查版本，磁盘缓存一天）、`src/usecase/dependency_updates.rs`，接入 `src/index/service.rs`、`src/ui/completion.rs` 和 `src/ui/file_editor/fixes.rs`。

| 功能 | 状态 |
|---|---|
| Ctrl+B / Ctrl+点击：build 脚本里的 `libs.androidx.core.ktx`、`libs.plugins.x`、`libs.versions.x`、`libs.bundles.x` 跳到 `libs.versions.toml` 的对应条目；toml 里 `version.ref = "x"` 跳到 `[versions]`，bundle 成员跳到 `[libraries]` | ✅ |
| 悬停：显示访问器、解析后的坐标（`androidx.core:core-ktx:1.13.1`，bundle 列出全部成员）和 catalog 文件 | ✅ |
| Find Usages（Alt+F7，在 toml 的键上）：分 Declarations、catalog 内引用、build 脚本引用三组；`libs.androidx.core` 不会误算成 `libs.androidx.core.ktx` 的用法 | ✅ |
| `libs.` 补全：在 build 脚本里逐段补全 catalog 访问器，条目显示坐标或版本 | ✅ |
| 新版本警告（AS 的 GradleDependency 检查）：build 脚本的 `"g:a:v"` 和 `id("x") version "v"`、catalog 的版本（`[versions]` 和内联版本）有更新时黄色波浪线，提示 “A newer version of g:a than v is available: w”；稳定版只推荐稳定版，预览版可推荐更新的预览版；动态版本不提示 | ✅ |
| Alt+Enter（Show Context Actions）：弹出所在行的快速修复，“Change to w” 直接改版本 | ✅ |
| 版本来源：androidx、com.android、com.google.android、Firebase 等查 Google Maven，其他查 Maven Central，插件查 Gradle Plugin Portal；结果缓存在 `~/.cache/Junction/maven-versions/`，一天后重查，失败一小时后重试 | ✅ |
| Project Structure 对话框（见第 61 节） | ✅ |
| Alt+Enter “Replace with new library catalog declaration for x”（AS 的 UseTomlInstead）：build 脚本里写死的 `"g:a:v"` 依赖和 `id("x") version "v"` 插件改成 `libs.x` / `alias(libs.plugins.x)`，同时在 `libs.versions.toml` 加 `[versions]` 和 `[libraries]`/`[plugins]` 条目（缺的段按 Gradle 顺序新建）；catalog 已有同坐标条目时改为 “Replace with existing version catalog reference 'x'” | ✅ |
| Groovy 脚本（`build.gradle`）里的 `libs.` 补全和跳转：与 Kotlin 脚本相同 | ✅ |
| `buildSrc` 常量（2026-10-22）：Kotlin 脚本里本来按索引解析；Groovy 脚本不建索引，另按名字解析：`Deps.coreKtx`、`Deps.Android.minSdk` 这样以类名开头的限定名 Ctrl+B 跳到 buildSrc（或工程里任一 Kotlin / Java 类）的成员，类名本身跳到类；`Deps.` 后补全该类的成员 | ✅ |
| buildSrc 常量的 Find Usages（2026-10-22）：在 `Deps.kt` 里 Alt+F7，结果多一组 “Groovy build scripts”，列出 `build.gradle` 里解析到该声明的 `Deps.coreKtx`（同名的 `libs.coreKtx` 等不算） | ✅ |
| buildSrc 里依赖字符串的新版本警告、Rename buildSrc 常量时同步改 Groovy 脚本 | ❌ 未做 |

Linux 实测（容器访问不了 dl.google.com，androidx 和 AGP 用预先写入的缓存，其他是实际联网结果）：build.gradle.kts 里 Ctrl+B `libs.androidx.core.ktx` 跳到 toml 第 9 行；toml 里所有旧版本都有警告，Alt+Enter 选 “Change to 1.13.1” 后版本被替换；`version.ref` 跳到 `[versions]`；Alt+F7 列出 build 脚本里的用法；`libs.andr` → `androidx` → `core.` → `ktx` 逐段补全。截图：reports/Gradle文件辅助-2026-10-15/。

## 21. 测试（2026-10-15）

代码：`src/domain/testing.rs`（找测试类和方法、选 Gradle 任务、结果树、状态行和时长格式）、`src/usecase/run_tests.rs`、`src/infra/gradle_tests.rs`（init script 逐条报告测试事件，跑完读 JUnit XML 报告）、`src/infra/gradle.rs` 的 `module_from_files`（没同步过时按 build 文件找模块），界面 `src/ui/test_service.rs`、`src/ui/test_view.rs`、`src/ui/file_editor/test_marks.rs`，Run 工具窗口的标签页在 `src/ui/run_views.rs`。

| 功能 | 状态 |
|---|---|
| gutter 运行图标：`src/test*`、`src/androidTest*` 里的 Kotlin / Java 文件，测试类显示 ▶▶、`@Test`（及 JUnit 5 的 `@ParameterizedTest` 等）方法显示 ▶；抽象类、companion object 不显示；嵌套类按 `Outer$Inner` | ✅ |
| 图标跟随上次结果：通过绿色对勾、断言失败黄色、异常红色（AS 的 `runSuccess` / `runFailed` / `runError`） | ✅ |
| 点击图标弹出 “Run 'ExampleUnitTest.addition_isCorrect'  Ctrl+Shift+F10”；Ctrl+Shift+F10（macOS ⌃⇧R）运行光标所在的方法，不在方法里时运行所在类 | ✅（弹窗有 Run、Debug、Run with Coverage；Modify Run Configuration 未做） |
| Gradle 任务：单元测试 `:app:testDebugUnitTest --tests a.ATest.method`（纯 JVM 模块 `:core:test`），仪器测试 `:app:connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=…`；变体跟随 Build Variants；没同步过时按最近的 build 脚本和 settings 脚本找模块 | ✅（仪器测试未在真机验证） |
| Run 工具窗口标签页：应用的 “app” 和测试运行（运行中图标为 Rerun，结束后为 JUnit），开始测试自动打开 Run 窗口并切到测试标签 | ✅ |
| 测试运行视图工具栏：Rerun、Rerun Failed Tests、Stop、Show Passed、Show Ignored、Sort Alphabetically、Sort by Duration、Expand All、Collapse All | ✅ |
| 状态行（IntelliJ 文案和配色）：“1 test failed, 2 passed, 1 ignored” + 灰色 “4 tests total, 25 ms”；运行中显示进度；停止时 “Stopped. … 1 / 3 tests”；编译失败 “Test events were not received” | ✅ |
| 结果树：Test Results › 测试类 › 测试，结果图标和时长（IntelliJ 的窄格式 “1 sec 234 ms”）；类可折叠；双击跳到源码 | ✅ |
| 右侧控制台：根节点显示 Gradle 输出，类节点显示类的输出和各测试的输出 / 失败，测试节点显示它的 stdout / stderr、断言两侧（“Expected :4 / Actual :5”）和堆栈 | ✅ |
| 断言两侧下方的 `<Click to see difference>` 链接（2026-10-20，照 IntelliJ）：点击在对比窗口打开 Expected / Actual 两段文本（标签页名 “CalcTest.adds (Expected vs Actual)”，只读）。Linux 实测 `assertEquals(3, first)` 打开 3 对 13 的对比 | ✅ |
| JUnit XML 报告：跑完读本次写入的 XML，补上事件里没有的测试（仪器测试只有 XML） | ✅ |
| 编译失败：错误交给编辑器标红（同 Make Project），弹 “Error running '…'” 通知 | ✅ |
| Project 窗口右键运行测试（2026-10-20，照 IntelliJ 的 AllInPackage / AllInDirectory 生产者）：测试源集里的包目录 “Run / Debug 'Tests in 'com.example''”（单元测试 `--tests 'com.example.*'`，仪器测试用 runner 的 `package` 参数）；源集、源根、`src` 和模块目录 “Run / Debug 'Tests in 'app''”（整个模块，`src` 和模块目录只跑单元测试，且只在有单元测试时出现）；测试文件 “Run / Debug 'CalcTest'”。Linux 实测：右键 `test/java/demo` 运行得到 CalcTest 的结果树，Debug 在断点停下 | ✅ |
| 临时运行配置（2026-10-20，照 IntelliJ 的 temporary configurations）：从 gutter、Ctrl+Shift+F10 或 Project 窗口跑测试，会生成同名的临时配置（如 “CalcTest.adds”、“Tests in 'app'”），标题栏下拉自动选中它，排在应用配置之后、最新的在上，最多 5 个；选中时标题栏的 Run / Debug / Stop 和 Run 菜单作用于这个测试。Linux 实测：gutter 运行 CalcTest.adds 后下拉选中它，点 Run 重跑，点 Debug 在断点停下，Stop 结束 | ✅（不能保存成永久配置，重启后不保留） |
| Run with Coverage（2026-10-21，照 IntelliJ 的 Coverage 和 AS 的 JaCoCo 运行器）：gutter 弹窗、Project 窗口右键、Run 菜单 “Run '…' with Coverage”、标题栏运行控件的 ⋮（More Actions）都能用。init script 给模块加 `jacoco` 插件，测试任务结束后（失败也算）由 `junctionCoverage` 任务写 XML 报告到 `build/junction-coverage/<任务>.xml`；Android 模块取当前变体的 javac / Kotlin 输出，去掉 R、BuildConfig、Manifest。仪器测试（2026-10-21）：init script 给构建类型打开 `enableAndroidTestCoverage`（AGP 7.3 以前是 `testCoverageEnabled`），跑 `connected<变体>AndroidTest` 后用 `--continue` 接着跑 `junctionCoverage`，读设备拉回的 `build/outputs/code_coverage/<变体>AndroidTest/connected/**/*.ec` | ✅（未在 Android 项目和真机上实测；报告任务在容器里用模拟的 `.ec` 验证过） |
| Coverage 工具窗口（右侧，第一次跑覆盖率时出现）：列 Element、Class, %、Method, %、Line, %、Branch, %，格式 “50% (1/2)”（向下取整）；包可折叠，只有一个包时自动展开；双击类打开源码；Hide Coverage 清掉报告和编辑器标记 | ✅（没有按包分层、Flatten Packages、排序、Generate Coverage Report） |
| 编辑器覆盖标记：被测代码的行左侧绿条（执行过）、黄条（有分支没走到）、红条（没执行），改动标记画在上面 | ✅（改了代码后不标过期） |
| 测试历史（2026-10-22，照 IntelliJ 的 SMTestRunnerResultsForm / TestResultsXmlFormatter / ImportTestsGroup）：每次跑完把结果按 IntelliJ 的格式（`<testrun>`、`<count>`、`<suite>`、`<test>`、`<output>`、`<diff>`）存为 `<缓存目录>/test-history/<项目>-<哈希>/<配置名> - yyyy.MM.dd at HHh MMm SSs.xml`，只留最近 10 个。测试工具栏 Test History（时钟图标）列出 “CalcTest (2026/10/22 06:43)”，最新的在上，点击在测试标签里打开（结果树、输出、失败、断言对比都还原；同名临时配置还在时 Rerun 可用）。Import Tests from File…（工具栏按钮和 Find Action）可打开 IntelliJ 导出的结果文件或 JUnit XML 报告，失败时弹 “Import Failed”。Linux 实测：真实 Gradle 跑 CalcTest 一次失败一次通过，历史里两条，打开较早那条显示 subtracts 失败、Expected 4 / Actual 5 和堆栈 | ✅（没有 IntelliJ 的 Export Test Results 为 HTML；Linux 容器里没有文件选择对话框，导入只用单元测试验证） |

Linux 实测（Xvfb）：JDK 21 + Gradle 8.14 的纯 Java 多模块项目（`:core`，JUnit 4，通过 / 断言失败 / 异常 / `@Ignore` 各一个），未同步直接运行：
- 打开 CalcTest.java，类上 ▶▶、三个 `@Test` 方法上 ▶；点类图标弹出 “Run 'CalcTest'  Ctrl+Shift+F10”，运行 `:core:test --tests com.example.calc.CalcTest`；
- Run 窗口打开测试标签，结果树 adds ✓ 28 ms、later 忽略、throwsError 红色、addsWrong 黄色，状态行 “2 tests failed, 1 passed, 1 ignored  4 tests total, 32 ms”；gutter 图标随之变为对应结果；
- 光标在 addsWrong 里按 Ctrl+Shift+F10 只跑这个方法，标签名 “CalcTest.addsWrong”；选中它，控制台从 “Expected :5 / Actual :4” 开始，堆栈截到 `CalcTest.addsWrong(CalcTest.java:17)`；
- Rerun Failed Tests 只重跑两个失败的测试，adds 的 gutter 仍为绿色；结束后 Stop、（无失败时）Rerun Failed Tests 置灰；
- 主代码有编译错误时：根节点错误图标、状态行 “Test events were not received”、控制台是 javac 报错，弹出 “Error running 'CalcTest.adds'” 通知。

Android 单元测试（`testDebugUnitTest`）和仪器测试未实测（容器里没有 Android SDK 和设备）。

## 22. Debugger（2026-10-15）

代码：`src/domain/debugger.rs`（类、方法行号表、线程、栈帧、值；断点行到代码位置、包和源文件匹配；IntelliJ 的值 / 栈帧 / 线程显示格式）、`src/infra/jdwp_client.rs`（JDWP 客户端：后台读线程分发回复和事件）、`src/usecase/debug_session.rs`（调试会话）、`src/infra/breakpoint_store.rs`（断点持久化），界面 `src/ui/debug_service.rs`、`src/ui/debug_view.rs`、`src/ui/file_editor/breakpoints.rs`。图标取自 intellij-community 的 expui（breakpoints、run、debugger、toolwindows）。

| 功能 | 状态 |
|---|---|
| 行断点：点行号或 Ctrl+F8（macOS ⌘F8）切换；红点画在行号位置（新 UI 样式），所在行淡红底；生效后变为带勾的 `breakpointValid`；跨会话保存在设置目录 `breakpoints.json`；编辑时随行移动 | ✅ |
| 断点属性弹窗：右键断点弹出 “CalcTest.java:34”，Enabled 勾选、Condition 输入框（回车或 Done 应用，点外面也应用）、Remove；禁用的断点为空心 `breakpointDisabled` 且行不着色，有条件的断点点上显示 “?” | ✅（More / Breakpoints 对话框、Suspend 的 All / Thread 选择未做） |
| 日志断点：属性弹窗照 IntelliJ 加 “Suspend execution” 和 Log 区（“"Breakpoint hit" message”、“Stack trace”、“Evaluate and log:” 加表达式框）；Shift+点行号直接加一个不挂起、记录命中消息的断点；不挂起的断点画成黄点（expui 的 `breakpointUnsuspendent` 系列）。命中时（条件成立后）往 Debug 的 Console 打印 IntelliJ 的原文：“Breakpoint reached at com.example.calc.CalcTest.totals(CalcTest.java:34)”、“Breakpoint reached” 加每帧 `\tat …`、表达式的值（字符串原文、对象 `toString()`；出错为 “Unable to evaluate the expression "…" : …”），不挂起就接着跑；跨会话保存 | ✅ |
| 条件断点：命中时在该帧求值条件，为 false 就继续运行；条件出错时照 IntelliJ 停下，并提示 “Failed to evaluate breakpoint condition '…'. Reason: …” | ✅ |
| 断点设到类上：已加载的类直接设；没加载的类用 ClassPrepare（`包名.*`）等它加载再设，并按 SourceFile 过滤；一行在多个方法里有代码（lambda、构造器）时每处都设 | ✅ |
| Debug 测试：测试 gutter 弹窗 “Debug 'CalcTest.adds'  Ctrl+Shift+F9”（macOS ⌃⇧D），Ctrl+Shift+F9 调试光标处的测试；Gradle 的测试 JVM 带 JDWP 挂起等待，Junction 连上、设断点后放行；测试结果照常进 Run 窗口 | ✅ |
| 执行点：暂停时打开源码并定位，所在行蓝底（IntelliJ 的 EXECUTIONPOINT 样式，无 gutter 箭头）；切换栈帧跟着跳 | ✅ |
| Debug 工具窗口（Alt+5）：会话名和状态；工具栏 Resume（F9）、Pause、Stop（Ctrl+F2）、Step Over（F8）、Step Into（F7）、Step Out（Shift+F8）、Run to Cursor（Alt+F9）；“Threads & Variables” 和 “Console” 标签 | ✅（F7 在调试暂停时是 Step Into，否则仍是 Diff 的 Next Difference） |
| Frames：线程下拉（`"Test worker"@1: SUSPENDED`，可切换线程）、栈帧 `adds:12, CalcTest (com.example.calc)`，JDK / JUnit / Gradle 帧灰显 | ✅ |
| Variables：`this` 和可见局部变量（按槽位顺序），IntelliJ 的值格式（`4`、`'a' 97`、`"hello"`、`null`、`{CalcTest@444}`、`{int[3]@56}`）和图标；对象展开为实例字段（含父类，隐藏静态和合成字段），数组展开前 100 个元素 | ✅ |
| 值的描述：类覆盖了 `toString()` 的对象显示 `{Point@12} "Point(3)"`，java.util 集合显示 `size = 3` 并展开为元素（Map 展开为条目），包装类型显示值 `{Integer@447} 1` | ✅ |
| 求值（新 UI 的 Variables 顶部输入框）：Enter 求值，结果作为第一行；Ctrl+Shift+Enter（macOS ⇧⌘↩）或 + 按钮加为 Watch；Watches 每次暂停 / 切帧重算，跨会话保存在 `watches.json`，行尾 × 删除 | ✅（Alt+F8 的独立 Evaluate 对话框、代码片段模式未做） |
| 表达式：局部变量、`this` 的字段、当前类的静态字段、类名（`Math`、同包类、`java.util.Collections`）；字段和 Kotlin 属性（无字段时走 `getX()` / `isX()`）、数组下标和 `length`、实例 / 静态方法调用（按参数个数沿继承链找，参数自动拓宽和装箱）、`+ - * / %`、比较、`&& || !`、字符串拼接（对象调 `toString()`）、自动拆箱；错误信息照 IntelliJ（“Cannot find local variable 'y'”、“Method threw 'java.lang.NumberFormatException' exception.”） | ✅（new、类型转换、instanceof、赋值、数组创建见下两行；lambda 未做：IntelliJ 要先把 lambda 编译成类再定义到被调试的 VM 里，Junction 没有编译器；Kotlin 扩展函数未做） |
| 求值的 new、类型转换、instanceof、赋值（2026-10-20）：`new Point(1, 2)`、`new java.util.ArrayList(list)`（ClassType.NewInstance，按实参选最合适的构造器；Kotlin 写法 `Point(1, 2)` 找不到同名方法时当作构造）；嵌套类按 `Outer.Inner` 或当前类里的简名找到；`(int) 3.9`、`(byte) 300`、`(double) a / 2` 照 Java 的截断和溢出；`(String) x`、Kotlin `x as String` 检查类型，不符时报 “java.lang.ClassCastException: class … cannot be cast to class …”；`instanceof` / Kotlin `is` / `!is` 沿父类和接口判断；赋值 `a = 5`、`p.x = 10`、`COUNT = 8`、`numbers[0] = 'a'`（局部变量、字段、静态字段、数组元素，按声明类型转换和装箱，类型不符报 “Incompatible types”）；含赋值的求值后 Variables 自动刷新；方法重载也改为按实参类型挑（`Math.max(a, 100)` 用 int 版、`list.remove(0)` 用 remove(int)） | ✅ |
| 求值的数组创建（2026-10-20）：`new int[3]`、`new String[n][]`、`new long[2][3][1]`、带初始化的 `new int[] {1, 'b', a}`、`new String[][] {{s}, {}, null}`，Kotlin 的 `intArrayOf(…)` 等和 `arrayOf(…)`（元素类型 Object）；照 IntelliJ 数组类未加载时的做法，用 `java.lang.reflect.Array.newInstance` 创建再逐个写入元素（按元素类型转换，类型不符报 “Incompatible types”，负长度报 NegativeArraySizeException）；方法重载按数组类型挑（`Arrays.toString(int[])` 不会选到 `long[]` 版） | ✅ |
| Debug 'app'：标题栏 Run 右边的 Debug 按钮、Shift+F9（macOS ⌃D）；构建、安装后以 `am start … -D` 启动（应用等调试器），通过 adb 的 `jdwp:<pid>` 连上进程，设断点后放行；Run 控制台显示 “Waiting for application to come online: …” | 🟡（只在模拟 adb 下验证：adb 服务端协议 + 真实 JVM，未在真机 / 模拟器验证） |
| Attach debugger：Device Explorer 进程页右键 “Attach debugger”，附加到已在运行的可调试进程 | 🟡（同上） |
| Disconnect：Debug 工具栏 Stop 右边的按钮（expui `exit` 图标），JDWP `VirtualMachine.Dispose` 后断开，程序去掉断点继续运行（测试照常跑完出结果）；Console 显示 “Disconnected from the target VM, address: '127.0.0.1:…', transport: 'socket'” | ✅ |
| Attach Debugger to Android Process 对话框（2026-10-21，`src/ui/attach_debugger_dialog.rs`，照 AS 的 `AndroidProcessChooserDialog`）：Run 菜单和 Find Action 打开 “Choose Process”；在线设备下列出可调试进程（`track-jdwp` 加 DDM `HELO`），默认只列项目的进程（进程名以项目 applicationId 开头），勾 “Show all processes” 列全部；非 0 用户显示 “(user N)”；对话框开着时每两秒刷新；记住上次选的设备和进程（重新打开时选中）；OK 或双击附加调试器。“Use Android Debugger Settings from” 和 “Debug Type” 只有 “[Use default settings]” 和 “Java Only”（没有原生调试）。实测（Xvfb，模拟 adb 服务端）：默认只列 com.example.psd.debug 的三个进程，勾选后多出 systemui；双击后 Debug 窗口开始附加 pid 12400 | 🟡（未在真机验证） |
| Run / Debug 时的 Logcat 联动（2026-10-22，照 AS 运行配置 Miscellaneous 页的 Logcat 选项和 `LogcatToolWindowFactory.showLogcat`）：Edit Configurations 新增 Logcat 区，两个选项默认都关（同 AS）。勾 “Show logcat automatically” 后，Run 或 Debug 启动应用后打开 Logcat 窗口，选中名为 “应用ID (设备序列号)” 的标签，没有就新建一个，读那台设备、过滤 `package:应用ID`。勾 “Clear log before launch” 后，在 `am start` 之前执行 `adb logcat -c`，正在读那台设备的 Logcat 面板一起清空（AS 的 `ClearLogcatListener`）。实测（Xvfb，假 adb）：Run 后出现标签 “com.example.run (emulator-5554)”，过滤 `package:com.example.run`；`logcat -c` 在 `am start` 前 0.3 秒；切到别的标签再 Run，会回到同一个标签而不新建，面板被清空 | ✅（未在真机验证） |

求值扩展的实测：JDWP 集成测试（真实 JVM）覆盖上面每个例子；界面上（Xvfb，JUnit 4 示例停在 `CalcTest.java:10`）在求值框输入 `first = (int) 2.9 + new java.util.ArrayList().size()`，结果 `= 2`，下面的 Variables 随即显示 `first = 2`。

求值实测（同一项目的 `totals` 测试，循环 `sum += items.get(i)`）：右键第 34 行断点输入条件 `i == 2` 回车，断点显示 “?”；Debug 后只在第三次循环停下（`sum = 3`、`i = 2`），`items = {ArrayList@437} size = 3`，展开为 `[0] = {Integer@447} 1` 等；输入 `items.get(i) * 10` 回车得 `30`；Ctrl+Shift+Enter 加 Watch `sum + i = 5`、`items.contains(7) = false`，重启后 Watch 仍在。另有对真实 JVM 的求值集成测试（算术、字符串、字段、getter、集合、Map、静态方法、异常、拆箱、条件）。

日志断点和 Disconnect 实测（同一 `totals` 测试）：第 34 行设不挂起的日志断点，勾 “"Breakpoint hit" message”、Evaluate and log `"sum = " + sum`，第 36 行普通断点；Debug 后 Console 依次打印三遍 “Breakpoint reached at com.example.calc.CalcTest.totals(CalcTest.java:34)” 和 `sum = 0` / `sum = 1` / `sum = 3`，然后停在第 36 行（`sum = 6`）；点 Disconnect，Console 显示 “Disconnected from the target VM, address: '127.0.0.1:37657', transport: 'socket'”，测试跑完并通过。Shift+点第 32 行出现黄点，`breakpoints.json` 记下 `suspend: false`、`log_message: true`。

Linux 实测（Xvfb，JDK 21 + Gradle 8.14 的 calc 项目，未同步）：点第 12 行行号设断点 → `adds` 的 gutter 弹窗选 “Debug 'CalcTest.adds'” → 约 20 秒（含 Gradle 守护进程启动）后在第 12 行暂停，断点变为已验证，Debug 窗口自动打开，线程 `"Test worker"@1: SUSPENDED`，`this = {CalcTest@444}`；F8 到第 13 行；F9 结束，Console 显示 “Connected to the target VM, address: '127.0.0.1:…', transport: 'socket'” 和 “Disconnected from the target VM”，Run 窗口 “1 test passed”。再用 Ctrl+Shift+F9 调试、F7 步入 `Calc.add`：自动打开 Calc.java 第 4 行，变量 `a = 2`、`b = 2`，栈帧 `add:4, Calc` / `adds:12, CalcTest`。JDWP 客户端另有对真实 JVM 的集成测试（断点、局部变量、数组和对象展开、Step Into / Over / Out）。

## 23. Gradle JDK（2026-10-15）

代码：`src/domain/jdk.rs`（JDK、设置值、按 AS 规则选择、`release` 和 `.gradle/config.properties` 解析、识别“找不到 Java”的输出）、`src/infra/jdk_discovery.rs`（查找本机 JDK）、`src/infra/gradle.rs`（每次调用 Gradle 时设置 `JAVA_HOME`），设置页在 `src/ui/settings_dialog.rs`。

| 功能 | 状态 |
|---|---|
| Settings › Build, Execution, Deployment › Gradle：每个 Gradle 构建一个 “Gradle JDK” 下拉，选项照 AS：Detect automatically、`GRADLE_LOCAL_JAVA_HOME`、`JAVA_HOME`、找到的每个 JDK（`jbr-21  JetBrains Runtime version 21.0.3  /path`）、Add JDK…（选目录并校验）；按构建保存在 `settings.conf` 的 `gradle_jdk_<构建>`，值与 AS 的 `gradleJvm` 相同（`#GRADLE_LOCAL_JAVA_HOME` / `#JAVA_HOME` / 路径） | ✅ |
| 自动选择：项目 `.gradle/config.properties` 的 `java.home`（AS 打开过的项目会写） → `JAVA_HOME` → AS 自带 JBR → 最新的 JDK 17+ → 最新的任何 JDK | ✅ |
| 查找位置：Windows 的 `Program Files\Android\Android Studio*\jbr`、`%LOCALAPPDATA%\Programs\Android Studio*`、JetBrains Toolbox（`%LOCALAPPDATA%\JetBrains\Toolbox\apps\AndroidStudio\ch-*\*`）、`Program Files` 下 Java / Eclipse Adoptium / Microsoft / Zulu / Amazon Corretto / BellSoft / Semeru；macOS 的 `/Applications/Android Studio*.app`、Toolbox、`/Library/Java/JavaVirtualMachines`、Homebrew；Linux 的 `/opt/android-studio*`、`~/android-studio*`、snap、Toolbox、`/usr/lib/jvm`；各平台的 `~/.jdks`、sdkman 和 PATH 上的 `java`；同一 JDK 的多个符号链接只列一次 | ✅（Windows / macOS 未实测） |
| Sync、Build、Run、测试、调试、保存后编译都用所选 JDK | ✅ |
| Gradle 报找不到 Java（`JAVA_HOME is not set…`、`Android Gradle plugin requires Java 17` 等）时弹 AS 的 “Invalid Gradle JDK configuration found”，带 “Open Gradle Settings” 直达设置页 | ✅ |
| Download JDK…（2026-10-16，照 IntelliJ 的 `JdkList.kt` / `JdkInstaller`；`src/domain/jdk_feed.rs`、`src/infra/jdk_download.rs`、`src/ui/jdk_download_panel.rs`）：Gradle JDK 下拉里的 “Download JDK…” 在该构建下展开面板：从 JetBrains 的 JDK 列表（`download.jetbrains.com/jdk/feed/v1/jdks.json.xz`，xz 解压）取本机系统和架构可用的版本（不含 Early-Access），Version 下拉（新到旧）、Vendor 下拉（该版本的厂商，Oracle OpenJDK 在前）、Location（默认 `~/.jdks/<名字>`，同 IntelliJ）；Download 显示 “Downloading… 135 of 215 MB (62%)”，下载后校验 SHA-256，tar.gz 保留文件权限、zip 在 Unix 上按条目设权限，去掉压缩包的根目录，检查 `bin/java`，失败时清理；装好后自动选为该构建的 Gradle JDK（OK 后保存）。HTTPS 改用系统证书库（ureq 的 native-certs），公司代理的自签证书也能用。实测（Xvfb）：下载 Oracle OpenJDK 27（215 MB）到 `~/.jdks/openjdk-27`，`java -version` 正常，Gradle JDK 变成 “openjdk-27 (27)” | ✅ |
| General Settings（2026-10-20）：Gradle user home（设置后作为 `GRADLE_USER_HOME` 传给每次 Gradle 调用，空着时占位显示当前生效的目录）；Offline work（每次 Gradle 调用加 `--offline`，Sync 控制台第一行显示 “· offline”） | ✅ |
| 每个构建的 Distribution（IntelliJ 的 “Gradle from”，2026-10-20）：Wrapper（显示 `gradle-wrapper.properties` 里 `distributionUrl` 的版本，如 “Wrapper (Gradle 8.14.3)”；没有 wrapper 时用 PATH 上的 gradle）或 Specified location（填 Gradle 安装目录，跑它的 `bin/gradle`，目录里没有时提示）；空着等于 Wrapper；Sync 控制台第一行显示 “· Gradle: /opt/gradle” | ✅ |
| 修复（2026-10-20）：设置页改的 Gradle JDK 点 Apply / OK 后没有保存（应用设置时漏拷了这一项） | ✅ |
| 修改 wrapper 的 Gradle 版本、Project Structure 对话框（SDK Location、AGP 版本等） | ✅（第 61 节） |

Linux 实测：去掉 `JAVA_HOME`、PATH 中不含 `java` 启动（即用户截图里的情况），Sync Now 成功，自动用了 `/usr/lib/jvm` 里的 JDK 21；设置页显示 “Detect automatically: java-1.21.0-openjdk-amd64 (21.0.12.1)”，下拉里三个指向同一 JDK 的符号链接只列一次；把 Gradle JDK 设成 `JAVA_HOME`（未设置）后同步，弹出 “Invalid Gradle JDK configuration found / JAVA_HOME is not set and no 'java' command could be found in your PATH. / Open Gradle Settings”，点链接直达 Gradle 设置页。

2026-10-20 Linux 实测：设置页搜 “offline” 直达 Gradle 页；勾 Offline work，Distribution 选 Specified location 填 `/opt/gradle`，Gradle JDK 选 `JAVA_HOME`，OK 后 `settings.conf` 里三项都在；Sync 的控制台第一行 “Gradle JDK: JAVA_HOME … · Gradle: /opt/gradle · offline”，同步成功。

## 24. Rename（2026-10-15）

| 功能 | 状态 |
|---|---|
| 编辑器里 Shift+F6，或右键 Refactor › Rename…：AS 的 Rename 对话框，标题 “Rename Class / Method / Variable…”，“Rename class 'Calc' and its usages to:” 加新名字输入框 | ✅ |
| 参数和局部变量：只改所在作用域里的声明和使用 | ✅ |
| 类、方法、属性等：先找到声明（本文件的声明或 Go to Declaration 的结果，库里的声明拒绝），再在项目所有已索引文件里找整词出现；注释和字符串里的出现默认不改，勾选 “Search in comments and strings” 后列出并一起改；Kotlin `"$name"` 模板里的算代码 | ✅ |
| 对话框列出每一处出现（文件:行 + 该行文本），可逐条取消勾选；底部 “N occurrences in M files” | ✅ |
| 新名字校验：Java / Kotlin 关键字、非法标识符时显示 AS 的 “'class' is not a valid identifier”，Refactor 不生效 | ✅ |
| 类名与文件名相同（`Foo.kt` / `Foo.java`）时 “Rename file 'Foo.java'”，默认勾选，用 `git mv` 改名，打开的标签页跟着改 | ✅ |
| 开始前保存所有编辑器，改完刷新索引、Git 状态，未修改的标签页从磁盘重载并保持光标 | ✅ |
| 光标不在名字上时提示 AS 的 “Cannot perform refactoring. Caret should be positioned at the name of element to be refactored.” | ✅ |
| 语义级精确匹配（同名的不同符号，例如别的类的 `add`）、Preview 窗口、Undo | ❌ 文本整词匹配，同名不同符号要在列表里手动取消勾选 |

Linux 实测（Xvfb，示例 Gradle 项目）：类 `Calc` 改为 `Calculator`，`Calc.java` 改名为 `Calculator.java`（Git 显示为重命名），测试里的 `Calc.add` 跟着改，勾选注释后 Javadoc 里的 `Calc` 也改了，标签页跟到新文件；局部变量 `sum` 改为 `total`，三处全改；方法 `add` 列出声明和两处调用；输入 `class` 时显示错误且不能提交。

## 25. Optimize Imports（2026-10-15）

| 功能 | 状态 |
|---|---|
| Java / Kotlin 文件里 Ctrl+Alt+O（Find Action 里也能找到 “Optimize Imports”），可 Ctrl+Z 一步撤销，光标留在原来的代码上 | ✅ |
| 删掉文件里没用到的 import、重复的 import、本包的 import、Java 的 `java.lang.*` 单类 import；`*` 导入保留；Kotlin 运算符和委托函数（`getValue`、`setValue`、`plus`、`invoke`、`contains` 等）不写名字也会用到，保留；`import a.B as C` 按别名判断 | ✅ |
| 按 IntelliJ 默认代码风格排序：Kotlin 为其他、`java.*`、`javax.*`、`kotlin.*`、别名导入，组间不空行；Java 为其他，空行，`javax.*` 与 `java.*`，空行，`import static` | ✅ |
| 结果用提示显示：“Removed 3 unused imports”、“Rearranged imports”、“Unused imports not found” | ✅ |
| Project 窗口对目录批量优化（2026-10-21，见第 11 节 Project 窗口右键） | ✅ |
| Settings › Editor › Auto Import 页（2026-10-22）：Java、Kotlin 各一个 “Optimize imports on the fly”（默认关，同 IntelliJ）；勾上后 Ctrl+S 保存时静默删未用的 import 并排序，光标在 import 块里（正在写 import）时不动；IntelliJ 是编辑过程中随高亮完成就做，这里改在保存时做，切标签等自动保存不触发 | ✅ |
| import 块中间夹注释时保留注释原位 | ❌ 未做（现在不改动这样的 import 块） |

“是否用到”按文件里出现的整词判断（注释里出现也算用到），不做类型解析：只在注释里提到的类，其 import 会被保留。

Linux 实测（Xvfb）：`Shapes.kt` 里 7 条 import（含重复的 `java.io.File`、本包的 `Calc`、未用的 `File`）优化后剩 `java.util.UUID`、`kotlin.math.PI`、`kotlin.math.max` 三条，光标仍在原行，Ctrl+Z 一步恢复。

## 26. Introduce Variable（2026-10-15）

| 功能 | 状态 |
|---|---|
| Kotlin / Java 文件里 Ctrl+Alt+V，或右键 Refactor › Introduce Variable…、Find Action | ✅ |
| 有选区时用选中的表达式（不是表达式时提示 “Selected block should represent an expression”）；没有选区时弹 AS 的 “Expressions” 列表，列出光标处由内到外的表达式，上下键移动时在代码里高亮对应表达式 | ✅ |
| 名字建议照 IntelliJ 的 KotlinNameSuggester：调用名 / 属性名去掉 `get` / `is` / `set` 前缀再取驼峰后缀（`user.getName()` → `name`），字面量按类型（字符串 `string` / `str` / `s` / `text`，整数 `i` / `n`，布尔 `bool` / `b`），最后是 `value`；与函数里已声明的名字冲突时加数字；关键字转义（`class` → `klass`） | ✅ |
| Introduce Variable 对话框：名字、建议名（点击即填入）、“Replace all occurrences (N occurrences)”（同一代码块里出现多次时，默认全部替换）、Kotlin 的 “Declare with var”、Java 的 “Declare final”；名字非法时提示并不能提交 | ✅ |
| 声明插在第一个被替换处所在语句之前，沿用该语句的缩进；表达式本身就是一条语句（如单独一行的 `foo()`）时整条语句变成声明；一次 Ctrl+Z 撤销 | ✅ |
| Java 的类型从表达式可见的信息推出：字面量、`new Foo<T>(…)`、强转、比较和逻辑运算（`boolean`）、含字符串的 `+`（`String`）；推不出时提示 “Cannot perform refactoring without a type” | ✅ |
| 不在代码块里（类属性初始化、表达式体函数 `fun f() = …`）时提示 “Cannot refactor in this place” | ✅ |
| AS 的就地模式（直接在代码里改名、各处同步）、“Specify type explicitly”、Java 方法调用结果的类型推断、把表达式体函数转成代码块 | ❌ 未做 |

Linux 实测（Xvfb）：Kotlin 里光标在 `user.getName()` 上按 Ctrl+Alt+V，弹出 Expressions 列表（`user`、`user.getName()`、`"Hello " + user.getName()`、`println(…)`），选第二项时代码里对应高亮；对话框建议 `name`、`value`，3 处出现默认全部替换；Refactor 后在第一处语句前插入 `val name = user.getName()`，三处都换成 `name`。

## 27. Introduce Constant（2026-10-15）

| 功能 | 状态 |
|---|---|
| Kotlin / Java 文件里 Ctrl+Alt+C，或右键 Refactor › Introduce Constant…、Find Action；选表达式的方式与 Introduce Variable 相同（选区或 “Expressions” 列表） | ✅ |
| Kotlin：只接受编译期常量（字面量、不含模板的字符串、它们的运算、`toInt()` / `shl` 等转换和位运算、本文件的 `const val`），否则提示 IntelliJ 的 “Can't introduce constant for non constant expression”；生成 `private const val NAME = …`，放在包含该表达式的顶层声明之前（KDoc 仍跟着原声明），后面空一行 | ✅ |
| Java：表达式不能用到局部变量、参数、`this` 或实例字段，否则提示 “Selected expression cannot be a constant initializer”；生成 `public static final 类型 NAME = …;`，放在最近的可含静态成员的类的第一个成员之前（IntelliJ 默认可见性 public） | ✅ |
| 名字照 IntelliJ：字符串常量先用其内容（`"Hello, world"` → `HELLO_WORLD`，NameUtil.capitalizeAndUnderscore），再是变量名建议的大写形式（`STRING`、`STR`、`I`…）；与文件中已有名字冲突时加数字 | ✅ |
| 同一文件里相同的表达式全部列为可替换处；“Replace all occurrences” 默认值照 AS：Kotlin 勾选，Java 不勾选（INTRODUCE_CONSTANT_REPLACE_ALL） | ✅ |
| 放进 `companion object` / 其他类、可见性选择、Java 前向引用时放到被引用字段之后、Kotlin 模板中的常量 | ❌ 未做 |

Linux 实测（Xvfb）：Kotlin 文件里光标在 `"Hello, world"` 上按 Ctrl+Alt+C，建议 `HELLO_WORLD`，两处出现全部替换，`private const val HELLO_WORLD = "Hello, world"` 插在带 KDoc 的函数之前；选 `"Hello, world" + name` 时提示 “Can't introduce constant for non constant expression”；Ctrl+Z 一步恢复。

## 28. Surround With（2026-10-15）

| 功能 | 状态 |
|---|---|
| Kotlin / Java 文件里 Ctrl+Alt+T，或右键 Surround With…、Find Action：AS 的 “Surround With” 弹窗，按 IntelliJ 的顺序和名字编号列出（1–9、0、A…），按编号直接套用，上下键 + 回车也可以 | ✅ |
| Kotlin 语句：if、if / else、run { }、try / finally、try / catch / finally、try / catch、while、do / while、for；Java 语句：if、if / else、while、do / while、for、try / catch、try / finally、try / catch / finally、synchronized、Runnable、{ } | ✅ |
| 包住选中的整行（没有选区时是光标所在行），里面缩进一级（按文件的缩进单位）；catch 照 IntelliJ 的默认：Kotlin `catch (e: Exception) { throw e }`，Java `catch (Exception e) { throw new RuntimeException(e); }` | ✅ |
| 选中一行中的一部分时，前面多出表达式项：Kotlin `!(expr)`、`"${expr}"`、`(expr)`，Java `(expr)`、`!(expr)` | ✅ |
| 套用后光标停在模板要填的地方（`if (|)`、`for (|; ; )`）或选中占位内容（`true`、`Exception`、`item in items`）；一次 Ctrl+Z 撤销 | ✅ |
| Live Template 式的占位跳转（Tab 到下一个）、把块内声明移到外面（MoveDeclarationsOutHelper）、按调用推断 catch 的异常类型、when / if 表达式等其余表达式模板、Surround with Live Template | ❌ 未做 |

## 29. Live Templates（2026-10-15）

| 功能 | 状态 |
|---|---|
| Kotlin / Java 文件里输入缩写按 Tab 展开（补全弹窗开着时，缩写完整输入的也直接展开，照 IntelliJ 的 Tab）；模板照 IntelliJ 的 Kotlin.xml / Java.xml：Kotlin 的 main、maina、maino、sout、serr、soutv、iter、ifn、inn、void、fun0–2、interface、singleton、closure、anonymous / object、exfun、exval、exvar、psvm 系列；Java 的 sout、serr、souf、soutv、psvm、fori、itar、iter、ifn、inn、St、psf、prsf、psfi、psfs、thr、else-if | ✅ |
| 适用位置照 IntelliJ 的上下文：语句、表达式、类成员、Kotlin 顶层和 `object` 体、Java 的 if 块之后（else-if）；注释和字符串里不展开 | ✅ |
| 展开后按 IntelliJ 的变量顺序逐个停留：当前变量加框，Tab / Enter 到下一个、Shift+Tab 回上一个，最后停在 `$END$`；Escape 提前结束 | ✅ |
| 同一变量的多处（`fori` 的 `i`、`soutv` 里的 `escapeString(EXPR)`、`closure` 的参数）随输入同步修改 | ✅ |
| 展开时按当前行缩进和括号层级重新缩进（IntelliJ 的 toReformat） | ✅ |
| Settings › Editor › Live Templates：按 Kotlin / Java 列出全部缩写和说明（可搜索） | ✅ |
| 模板出现在代码补全列表里（2026-10-20，照 IntelliJ 的 `LiveTemplateCompletionContributor`）：缩写以已输入的词开头、当前位置适用的模板列出来，右侧是描述；和缩写完全一致的排第一，其余排在名字同样以这个词开头的符号之后；选中后直接展开并进入模板变量编辑。Linux 实测：`so` 列出 sout / soutv，`sout` 回车得到 `println()`，`soutv` 回车后变量同步编辑 | ✅ |
| 自定义模板（2026-10-20，照 IntelliJ 的 User 组）：Settings › Editor › Live Templates 里 User 组 + / − 新建、删除；选中后编辑 Abbreviation、Description、Template text（`$NAME$` 变量按出现顺序停留，同名的多处同步，`$END$` 是结束位置），每个变量的默认值，以及适用位置（Kotlin：Statement、Expression、Class、Top-level、Object declaration；Java：Statement、Expression、Declaration）；每个模板（含内置模板）前的勾选框可关闭它；Apply / OK 后存到设置目录的 `live-templates.json`，下次启动读回。自定义模板与内置模板同名时优先 | ✅ |
| IntelliJ 根据代码计算的变量（`kotlinVariable()`、`suggestIndexName()`、`iterableComponentType()` 等）用固定默认值（`i`、`iterable`、`Object`…）；soutp、soutf 等依赖这些计算的模板未收录；自定义模板的变量表达式（只有默认值）、模板分组 / 导入导出、Surround with Live Template | ❌ 未做 |

Linux 实测（Xvfb，自定义模板）：新建 `pv`，文本 `System.out.println("$NAME$ = " + $NAME$);$END$`，NAME 默认值 `value`；Java 方法体里输入 `pv` 按 Tab（补全弹窗开着）展开，输入 `sum` 两处同步，得到 `System.out.println("sum = " + sum);`；关掉 Java 的 `sout` 后按 Tab 不再展开；`live-templates.json` 内容正确。

Linux 实测（Xvfb）：Kotlin 函数体里输入 `iter` 按 Tab 得到 `for (i in iterable) {}`，`iterable` 加框；输入 `items`、Tab、输入 `n`、Tab 后光标进入循环体；`soutv` 展开后把 `expr` 改成 `n`，字符串里的副本同步变成 `n`。

## 30. Generate（2026-10-15）

| 功能 | 状态 |
|---|---|
| Kotlin / Java 类里 Alt+Insert（macOS ⌘N），或右键 Generate…、Find Action：AS 的 “Generate” 弹窗；Kotlin 列 equals() and hashCode()、toString()（`object` 只有 toString()；interface / enum / annotation 不可用），Java 列 Constructor、Getter、Setter、Getter and Setter、equals() and hashCode()、toString() | ✅ |
| 选字段对话框（IntelliJ 的标题，默认全选，Select All / Select None）；Java 的 equals / toString / 构造函数不含 static 字段，Getter / Setter 只列还没有的（final 字段无 setter），没有可选时提示 IntelliJ 的 “No fields without getter were found” 等 | ✅ |
| 生成代码照 IntelliJ 默认模板：Kotlin 的 singleExpr.vm（`"Person(name='$name', age=$age)"`，数组 `contentToString()`）、defaultEquals.vm / defaultHashcode.vm（`javaClass` 比较、`other as Person`、基本类型属性排前、数组 `contentEquals` / `contentHashCode`）；Java 的 DefaultConcatMember.vm、java.util.Objects 版 equals / hashCode（`Double.compare`、`Objects.deepEquals`、`Objects.hash`）、defaultGetter / defaultSetter（boolean 用 `is`）、构造函数 | ✅ |
| 插在光标所在成员之后（否则类体开头），空行隔开；没有类体的 Kotlin 类补上 `{ }`；Java 需要的 `java.util.Objects` / `Arrays` import 自动加上 | ✅ |
| 已有 toString() / equals() 时先问 IntelliJ 的 “… are already defined for class … Do you want to delete them and proceed?”，确认后替换 | ✅ |
| Override / Implement Methods（需要父类成员）、Kotlin Secondary Constructor、Test…、equals 的父类 `super.equals`、toString 模板切换 | ❌ 未做 |

Linux 实测（Xvfb）：Java 类 Alt+Insert → equals() and hashCode()，生成 Objects 版 equals / hashCode 并加上两个 import；toString() 生成后再生成一次，弹出 “Method 'String toString()' is already defined…”，确认后替换；Kotlin 类 `Point(val x: Int, val y: Int)` 生成与 IntelliJ 一致的 equals / hashCode。

## 阶段

- **M1 骨架**：玻璃窗口、标题栏、工具窗口布局、Log（分支树 + 提交图 + 详情）、Commit 面板基础提交、分支弹窗基础、Console。
- **M2 日常操作**：Diff 查看器、暂存区、Stash、Fetch/Pull/Push 对话框、分支全部动作、Reset/Tag、通知与快捷键、设置页。
- **M3 历史改写与冲突**：交互式 Rebase、Merge/Rebase/Cherry-pick 对话框、三方合并工具、Blame、文件历史、分支比较。
- **M4 进阶**：部分提交、多 changelist、Shelf、补丁、Clone、GPG。
- **M5 多仓库**：多根、Worktree、Submodule。
- **M6 托管平台**：GitHub/GitLab Pull Requests。
- **M7 代码索引**：多语言符号索引、跨语言桥接、LSP、Go to Declaration / Find Usages / Go to Symbol。
- **M8 搜索**：Find / Replace in Files、Search Everywhere、Find Action、Recent Files、File Structure、Go to Line。

## 31. Translations Editor（2026-10-16）

Android Studio 的 Translations Editor。代码：`src/domain/translations.rs`（解析 `<string>`、Android 转义、`values-fr` / `values-zh-rCN` / `values-b+sr+Latn` 对应的语言区域和 AS 的列名、改 / 加 / 删字符串和 `translatable`）、`src/usecase/translations.rs`（汇总成表、各项编辑）、`src/infra/res_files.rs`（res 目录读写）、`src/ui/translations_editor.rs`。

| 功能 | 状态 |
|---|---|
| 打开：values 目录下含 `<string>` 的 XML 顶部显示 AS 的横幅 “Edit translations for all locales in the translations editor.”，“Open editor” 打开、“Hide notification” 隐藏；Find Action “Open Translations Editor”（当前文件不在 values 目录时打开项目第一个模块的 `src/main/res`）；作为编辑区标签页 “Translations Editor”，打开前先保存所有编辑器 | ✅ |
| 表格：Key、Resource Folder（`app/src/main/res`）、Untranslatable 勾选框、Default Value，每个语言一列（列名 “Chinese (zh) in China (CN)”、“French (fr)”，按名称排序）；值按 Android 规则反转义显示（`\'` → `'`、`&amp;` → `&`），带标签的值原样显示；缺翻译的单元格淡红底，缺翻译的键红字，不可翻译的键不算缺；`values-night` 等非语言目录不计；表格可横向滚动 | ✅ |
| 编辑：点单元格选中，下方 Key / Default Value / Translation 显示内容，Translation 右侧注明语言；双击单元格直接编辑下方对应字段，回车或离开字段时写回 XML（转义 `'` `"` `&` `<`、开头的 `@` `?`，首尾空格加引号），清空某语言的翻译即删除该条；没有该语言目录时新建 `values-xx/strings.xml`；写入后打开的编辑器重新加载、重新索引；编辑器里保存 values 文件时表格跟着刷新 | ✅ |
| 工具栏：+ Add Key（Key、Default Value、Resource Folder 对话框，照 AS 校验 “Key cannot be empty”、“Key 'x' already exists”、非法字符）、− Remove Keys（选中的键，Ctrl / Shift 点击多选，确认后从所有语言删除）、Add Locale（语言下拉，新建 `values-xx/strings.xml` 并放入 `app_name`，不可翻译时放第一个可翻译的键，同 AS 不留空目录）、Show All Keys / Show Translatable Keys / Show Keys Needing Translations 过滤、Reload | ✅ |
| 多个 res 目录（flavor、多模块）合并显示、Key 列冻结、单元格内直接编辑、`plurals` / `string-array`、Order a translation、按语言过滤列、删除时查找引用（Safe Delete） | ❌ 未做 |

Linux 实测（Xvfb，含 values、values-fr、values-zh-rCN、values-night 的示例项目）：打开 `values/strings.xml` 出现横幅，点 Open editor 打开表格，settings 的法语和 welcome 的中法两列标红；双击 settings 的法语格输入 `Parametres d'app` 回车，`values-fr/strings.xml` 写入 `Parametres d\'app`，该行恢复正常；Add Key 输入已存在的 `hello` 提示 “Key 'hello' already exists”，改为 `about` 后写入并选中；Show Keys Needing Translations 只剩 about、welcome；Remove Keys 确认后 about 从文件删除；Add Locale 选 German 新建 `values-de/strings.xml`，出现 “German (de)” 列，其余键变红；勾选 welcome 的 Untranslatable 写入 `translatable="false"`。

## 32. Vector drawable 预览（2026-10-16）

Android Studio 打开 `res/drawable*/`、`mipmap*/` 下的 vector XML 时的预览。代码：`src/domain/vector_drawable.rs`（VectorDrawable → SVG），界面 `src/ui/file_editor/vector_preview.rs`，gutter 图标在 `src/ui/file_editor/previews.rs`。

| 功能 | 状态 |
|---|---|
| 编辑器右上角 Code / Split / Design 三个按钮（默认 Split）：Split 在 XML 右侧显示预览，Design 只显示预览（不显示 gutter），Code 只有 XML；预览按长边 256 px 缩放，下方注明 “24 × 24 dp”；编辑 XML 时即时重画；无法渲染时显示原因 | ✅ |
| 转换：`pathData`（与 SVG 路径语法相同）、`fillColor` / `strokeColor`（`#RGB` `#ARGB` `#RRGGBB` `#AARRGGBB`、`@color/…` 按项目资源解析、`@android:color/white` 等、`?attr/…` 按默认主题近黑色）、`fillAlpha` / `strokeAlpha`、`strokeWidth`、`strokeLineCap` / `strokeLineJoin` / `strokeMiterLimit`、`fillType="evenOdd"`；`<group>` 的 pivot / rotation / scale / translate（Android 的变换顺序）、`<clip-path>`；`<aapt:attr>` 里的线性 / 径向渐变（`<item>` 或 start / center / endColor，sweep 用首色）；`<vector>` 的 `android:tint`、`android:alpha` | ✅ |
| gutter 预览：布局 XML 的 `@drawable/ic_star`、代码里的 `R.drawable.ic_star` 指向 vector 时，行号旁画出缩略图（以前只有位图） | ✅ |
| shape / selector / layer-list / animated-vector 预览、trimPath、主题属性按真实主题解析、预览缩放和背景切换 | ❌ 未做 |

Linux 实测（Xvfb）：打开 `drawable/ic_star.xml`（填充 `@color/accent` 的五角星，里面一个按 pivot 缩放 0.4 的白色圆），右侧显示橙色星和白色圆，注明 “24 × 24 dp”；把圆的 `#FFFFFF` 改成 `#D50000`，预览立即变红；点 Design 只显示预览、gutter 色块和改动标记不再显示；打开 `layout/main.xml`，`android:src="@drawable/ic_star"` 那一行的 gutter 出现星形图标。

## 33. Reformat Code（2026-10-16）

IntelliJ 的 Code › Reformat Code（Ctrl+Alt+L，macOS ⌘⌥L），Kotlin / Java。代码：`src/domain/reformat.rs`，编辑器入口 `FileEditor::reformat_code`；右键菜单和 Find Action 里也有。

| 功能 | 状态 |
|---|---|
| 范围：有选区时只格式化选中的行，否则整个文件；一次可撤销的编辑，光标留在原来的代码上；提示 IntelliJ 的 “Formatted N lines” / “No lines changed: content is already properly formatted” | ✅ |
| 缩进：按所在括号重新缩进（单位取文件已有的缩进，2 空格文件保持 2 空格，否则 4 空格，Tab 文件用 Tab）；Kotlin 官方风格的参数、链式调用、`=` 后表达式、`?:`、`when` 分支的 `->` 续一级，`&&` / `\|\|` 续两级，同一语句里并列的续行对齐，嵌套续行再深一级；属性的 `get()` / `set()` 在属性下一级；类头 `:` 后的父类列表续一级而类体按类缩进；Java 续行 8、`case` 体在标签下一级、方法声明多行参数与第一个参数对齐（IntelliJ 的 ALIGN_MULTILINE_PARAMETERS），方法体按声明行缩进 | ✅ |
| 空格：`,` 后加空格；`if` / `for` / `while` / `when` / `catch` / `switch` 后加空格；`{` 前加空格（`){`、`else{`、`foo{`，数组初始化 `]{` 除外），`}else` / `}catch`；赋值、`==` / `!=` / `===`、`&&` / `\|\|` 两侧单个空格；比较 `<` `>` `<=` `>=` 两侧加空格（只在两边是数字或小写名字时，避免误伤泛型 `List<Int>`）；Kotlin 冒号 `name: Type`、`fun f(): Int`，父类 / 构造函数委托 / `object :` / 类型参数上界 `<E : Any>` 冒号前加空格；多个空格合成一个，行尾空格去掉 | ✅ |
| 空行最多保留 2 行（IntelliJ 默认 KEEP_BLANK_LINES_IN_CODE = 2）；字符串、字符、注释、Kotlin 原始字符串 / Java 文本块、`${…}` 模板内容原样保留；块注释的 ` * ` 行跟着注释对齐 | ✅ |
| 换行（WRAP_AS_NEEDED、右边界 120）、import 优化联动、Rearrange code、XML / Gradle 文件格式化、`${…}` 里的表达式加空格、代码风格设置页、`.editorconfig` | ❌ 未做 |

验证：除单元测试外，用 intellij-community 的 82 个已格式化源文件（Kotlin 官方风格和 IntelliJ 自己的 2 空格 Java）做回归：69 个完全不变，剩下的差异基本是源文件里本来就没格式化的行（混入的 2 空格行、多余空格）和 IntelliJ 项目自定义的对齐（测试 `domain::reformat::corpus`，`JUNCTION_FMT_DIR=目录 cargo test corpus -- --ignored --nocapture`）。Linux 实测（Xvfb）：一个缩进混乱的 Kotlin 类按 Ctrl+Alt+L 后变为 4 空格缩进、`fun total(): Int {`、`if (i > 0 && i < 100) {`、`} else {`、`.filter { it > 1 }` 链在 `items` 下一级、多余空行减到 2 行，光标仍在原行。

## 34. Parameter Info（2026-10-16）

IntelliJ 的 View › Parameter Info（Ctrl+P，macOS ⌘P），Kotlin / Java。代码：`src/domain/parameter_info.rs`（找光标所在的调用和当前实参、从声明源码取参数列表、当前参数和不匹配的重载），界面 `src/ui/file_editor/parameter_info.rs`。

| 功能 | 状态 |
|---|---|
| 光标在调用的括号里按 Ctrl+P：用 Go to Declaration 找到所有声明（重载、项目和外部库源码），弹窗显示在 `(` 上方，每个重载一行，参数照源码写法（Kotlin `name: String, times: Int = 1`、Java `int a, int b`，去掉 `val` / `var` / 可见性修饰），没有参数显示 `<no parameters>` | ✅ |
| 当前参数加粗：按光标前的逗号数（跳过字符串、字符、注释、内层括号和 lambda），Kotlin 命名实参（`times = `）按名字，vararg 收下多余的实参；实参比参数多的重载灰显 | ✅ |
| 跟随光标：移动或输入时更新加粗的参数，光标离开这次调用就关闭，Esc 关闭（补全菜单打开时 Esc 先关补全） | ✅ |
| 自动弹出：在名字后输入 `(` 1 秒后（IntelliJ 的 Autopopup 默认 1000 ms）光标仍在这次调用里就自动显示 | ✅ |
| 按实参类型选中最匹配的重载、文档注释、`super(` / `this(` 委托、Java `new` 匿名类的构造函数、注解参数 | ❌ 未做 |

Linux 实测（Xvfb）：Kotlin 两个 `greet` 重载，光标在 `greet("a", true)` 的第二个实参上按 Ctrl+P，弹出两行，`times: Int = 1` 和 `loud: Boolean` 加粗；按 End 离开调用后关闭；新输入 `greet(` 后约 1 秒自动弹出。Java 的 `Calc.add(2, 2)` 显示 `int a, int b`；`assertEquals(` 显示 JUnit 源码里的 12 个重载。

## 35. Alt+Enter：Import class 与意图操作（2026-10-16）

IntelliJ 的 Show Context Actions（Alt+Enter）在问题修复之外，补上了 Import class 和不需要类型信息的意图操作（Kotlin / Java）。代码：`src/usecase/import_class.rs`、`src/domain/intentions.rs`，弹窗 `src/ui/file_editor/fixes.rs`。

| 功能 | 状态 |
|---|---|
| Import class 'Item'：光标在文件看不到的类名上（不在同包、没有 import、不是隐式导入的 `java.lang` / `kotlin` 等、不是文件自己的声明或局部变量、不是 `x.Name` 成员），从项目和外部库找同名类；只有一个时直接导入，多个时弹出 IntelliJ 的 “Class to Import” 选择列表；import 按 IntelliJ 的位置和顺序插入，光标留在原处 | ✅ |
| Convert to block body / Convert to expression body（Kotlin）：`fun f(x: Int): Int = x * 2` ↔ `fun f(x: Int): Int {\n    return x * 2\n}`（`: Unit` 时不加 `return`）；没写返回类型时不提供转为块体（块体的类型会变成 Unit）；函数体有多条语句时不提供转为表达式体 | ✅ |
| Add braces to 'if' / 'else' / 'for' / 'while' statement、Remove braces from … statement（Kotlin / Java）：语句体在同一行或下一行；同一行的 `if (a) b() else c()` 只给 `if` 部分加；只有一条语句且不是声明时才能去掉括号 | ✅ |
| Put arguments / parameters on separate lines、Put arguments / parameters on one line（Kotlin）：至少两个实参或参数；每个一行，`)` 单独一行；字符串里的逗号不拆 | ✅ |
| 需要类型的意图（Specify type explicitly、Add names to call arguments、Convert to string template 等）、Import function / 扩展函数、自动导入提示（“? java.util.Date? Alt+Enter”）、意图设置页 | ❌ 未做 |

Linux 实测（Xvfb）：`com.example.a.Item` 和 `com.example.b.Item` 两个同名类，光标在 `Item()` 上 Alt+Enter 出现 “Import class 'Item'”，回车后弹出 “Class to Import” 两项，选第二个后插入 `import com.example.b.Item`；`if (n > 1) println(n)` 上 Alt+Enter → “Add braces to 'if' statement” 变为三行块；`fun price(n: Int): Int = n * 2` 上 → “Convert to block body” 变为 `return n * 2` 的块体。

## 36. Override / Implement Methods（2026-10-16）

IntelliJ 的 Code › Override Methods（Ctrl+O）和 Implement Methods（Ctrl+I，macOS 同为 ⌃O / ⌃I），Kotlin / Java；Generate（Alt+Insert）弹窗末尾也有 “Override Methods…” / “Implement Methods…”，Find Action 里也能找到。代码：`src/domain/overrides.rs`（类头的父类型和类型实参、成员能否覆盖 / 是否抽象、按 IntelliJ 模板生成）、`src/index/overrides.rs`（沿继承层级收集成员）、`src/ui/file_editor/overrides.rs`。

| 功能 | 状态 |
|---|---|
| 收集：从光标所在类的类头读父类型（Kotlin `:` 后、Java `extends` / `implements`），在项目和外部库源码里沿继承层级逐层找（近的优先，所以中间类已经覆盖的接口方法不再算抽象），类型参数换成实参（`Base<Int>` 里的 `accept(item: T)` 显示为 `accept(item: Int)`），最后加上 Kotlin `Any` / Java `Object` 的 equals、hashCode、toString（Java 还有 clone）；类里已有的同名成员不列 | ✅ |
| 规则：Kotlin 类成员默认 final，只有 `open` / `abstract` / `override` 和接口成员可覆盖，接口里没有函数体（或属性没有初始值）的是抽象的；Java 排除 `private` / `static` / `final`，接口里非 `default` 的是抽象的；Implement 只列抽象成员，Override 只列有实现的 | ✅ |
| 选择对话框：Kotlin “Implement Members” / “Override Members”，Java “Select Methods to Implement” / “Select Methods to Override”；每项显示签名（Kotlin `area(scale: Double): Double`、Java `compute(int, String):int`）和所属类，Implement 默认全选、Override 默认不选，Select All / Select None；没有可选时提示 “No members to implement have been found” 等 | ✅ |
| 生成（IntelliJ 默认模板）：Kotlin 抽象成员 `override fun …` 里写 `TODO("Not yet implemented")`，有实现的写 `return super.f(…)`（Unit 不写 `return`，vararg 传 `*xs`），属性生成 `override val x: T` + `get() = …`（var 还有 `set(value)`），去掉参数默认值，保留 `suspend` / `operator` / `protected` 等修饰；Java 加 `@Override`，接口方法为 `public`，抽象的返回默认值（`return 0;` / `false` / `null`，void 为空），有实现的 `return super.f(…);`，保留 `throws`；插在光标所在成员之后，一次可撤销的编辑 | ✅ |
| 签名里类型的 import、同名不同参数的重载区分（按名字排除已有成员）、Kotlin `super<Interface>` 限定、Copy JavaDoc、按类分组的树形列表、Insert @Override 选项 | ❌ 未做 |

Linux 实测（Xvfb）：接口 `Shape`（`val label`、`fun area(scale: Double = 1.0)`、有默认实现的 `describe()`）、抽象类 `Base<T> : Shape`（`abstract fun accept(item: T)`、`open fun reset()`）、`class Circle : Base<Int>()`；在 Circle 里 Ctrl+I 列出 `accept(item: Int) Base`、`label: String Shape`、`area(scale: Double): Double Shape` 且全选，确定后生成三段 `TODO("Not yet implemented")`；Ctrl+O 列出 `reset()`、`describe()`、`equals`、`hashCode`、`toString`，选 describe 和 toString 后生成 `return super.describe()` / `return super.toString()`。

## 37. 语言服务器：语义 Rename 与 Alt+Enter 代码操作（2026-10-16）

装了语言服务器的语言（Kotlin 的 JetBrains `kotlin-lsp`、Java 的 `jdtls`、Rust 的 `rust-analyzer` 等，按 Settings 里的 Language Servers 配置，默认在 PATH 上找到就用；可关闭或改命令）在跳转 / Find Usages / 悬停之外，补上了需要类型信息的两项。代码：`src/index/lsp.rs`（rename、codeAction、codeAction/resolve、executeCommand、workspace/applyEdit、publishDiagnostics）、`src/index/service.rs`（`server_references`、`code_actions`）、`src/ui/file_editor/fixes.rs`。

| 功能 | 状态 |
|---|---|
| Rename（Shift+F6）语义化：有语言服务器时，对话框里的代码出现位置改用服务器的 `textDocument/references`（只列这个符号，不再整词匹配同名的其他符号），注释和字符串里的仍由索引给出供勾选；服务器还在索引时最多等 20 秒；没有服务器时照旧按索引整词匹配 | ✅ |
| Alt+Enter 加入服务器的代码操作：先显示编辑器自己的修复和意图，服务器的 `textDocument/codeAction`（快速修复、重构）返回后追加到列表（原本没有项时直接弹出）；服务器标记的首选修复和 quickfix 排在前面；请求带上服务器发布的该行诊断 | ✅ |
| 应用：操作自带的编辑、`codeAction/resolve` 后的编辑、或执行其命令时服务器通过 `workspace/applyEdit` 发来的编辑；当前文件作为一次可撤销的编辑（光标随之移动），其他文件写盘后重新加载、重新索引 | ✅ |
| 服务器诊断显示在编辑器和 Problems 里、补全改用服务器、Inline / Extract Method 等需要对话框的重构、文件创建 / 重命名类的 workspace edit、Code › Optimize Imports 走服务器 | ❌ 未做 |

Kotlin 说明：Junction 默认不常驻 Kotlin 语言服务（省 1–2 GB 内存）；要语义 Rename 和类型相关的快速修复，安装 JetBrains 的 kotlin-lsp（github.com/Kotlin/kotlin-lsp，或 VS Code / Open VSX 上 “Kotlin by JetBrains” 扩展的 server-bundle.json 里给出的 kotlin-server 压缩包）并让 `kotlin-lsp` 或新版的启动器 `intellij-server`（压缩包里的 `bin/intellij-server`，`kotlin-lsp.sh` 已标为弃用）在 PATH 上，或在 Settings 的 Language Servers 里填它的启动命令。kotlin-lsp 先用 Gradle 导入工程（首次约 40 秒，要能下载 Gradle 和插件），导入前只认得当前文件，Rename 和快速修复会不完整；导入失败（如 AGP 插件下载不到）时文件不在任何模块里，没有诊断也没有 Import 修复。

Linux 实测（Xvfb，kotlin-server 263.6379.0）：Kotlin/JVM Gradle 工程里 `val item = Item()`（`demo.a.Item` 和 `demo.b.Item` 两个同名类）上 Alt+Enter，在编辑器自己的 “Import class 'Item'” 之后列出服务器的 “Remove invocation”、“Import class 'Item' → demo.a.Item” / “→ demo.b.Item”、“Extract to local variable”、“Organize imports”，选 demo.b 后文件加上 `import demo.b.Item`；Shift+F6 重命名 `Shop.price` 为 `cost`，对话框列出 3 处（声明、`price(n)`、`Shop().price(3)`），`Other` 里的同名方法 `price()` 及其调用不在其中（整词匹配会列出 5 处），应用后只改这 3 处。

Linux 实测（Xvfb，rust-analyzer 1.97）：Rust 项目里 Shift+F6 重命名函数 `compute`，对话框列出 main.rs 两处和 util.rs 的 `super::compute(x)`，而 util.rs 里另一个函数的同名参数 `compute` 没有列入（整词匹配会列入）；`BTreeMap::new()` 上 Alt+Enter 出现服务器的 “Import `std::collections::BTreeMap`” / “Qualify as …”，回车后文件顶部加上 `use std::collections::BTreeMap;`；`compute(2)` 的 `2` 上出现 “Extract into variable” 等重构，选中后（经 codeAction/resolve）生成 `let x = 2;` 并替换。

## 38. Debugger：Evaluate Expression 对话框与 Breakpoints 对话框（2026-10-16）

照 IntelliJ 的 `XDebuggerEvaluationDialog` 和 `BreakpointsDialog`。代码：`src/ui/evaluate_dialog.rs`、`src/ui/breakpoints_dialog.rs`、`src/domain/expression.rs`（`expression_at`）、`src/ui/debug_service.rs`（`evaluate_task`、`children_task`、`all_breakpoints`、`remove_breakpoint`、`show_source`）；变量树的行由 `debug_view::var_row` 共用。

| 功能 | 状态 |
|---|---|
| Evaluate（Alt+F8，暂停时在编辑器里；或 Debug 工具栏的计算器按钮，带上求值输入框里的内容）：表达式框预填选中文本，没有选中时取光标处的表达式（IntelliJ 的 `getExpressionRangeAtOffset`：`point.x` 在 `x` 上取整个限定引用，`list.get(i)` 在 `get` 上取整个调用连同参数，`a[2].length`、`user?.name` 同理；关键字和数字上不预填） | ✅ |
| Evaluate（Enter 或按钮）在当前帧求值，对话框保持打开；Result 里是 `result = {ArrayList@445} size = 3` 这样的树，对象 / 数组 / 集合可逐层展开，错误显示为红字（“Cannot find local variable 'nope'”）；未暂停时提示 “The debugger is not paused” | ✅ |
| 历史（时钟按钮）：本次运行求值过的表达式，最新的在前，最多 20 条，点选填回；Add to Watches 把表达式加进 Debug 窗口的 Watches | ✅ |
| Breakpoints（Ctrl+Shift+F8 / ⇧⌘F8，Run › View Breakpoints…，或断点弹窗里的 “More (Ctrl+Shift+F8)”，后者选中该断点）：左边按语言分组（Java / Kotlin Line Breakpoints）列出所有行断点，每项有启用复选框、和编辑器 gutter 一致的图标（红 / 黄日志断点、禁用空心、已验证打勾）、`CalcTest.java:34` 和该行源码 | ✅ |
| 右边是选中断点的属性：Enabled、Suspend execution、Condition、Log（“Breakpoint hit” message、Stack trace、Evaluate and log），勾选立即生效，文本在离开输入框或回车时生效，运行中的调试会话同步更新；工具栏 − 删除（选中下一个）；View Source 或双击打开该行 | ✅ |
| Java Exception Breakpoints（2026-10-16）：Breakpoints 对话框里的 “Java Exception Breakpoints” 组，固定有 IntelliJ 默认关闭的 “Any exception”；+ 输入异常类全名添加（如 `java.lang.IllegalStateException`，含其子类）；属性 Enabled 和 Notifications（Caught exception / Uncaught exception）；存到 `exception-breakpoints.json`，运行中的会话同步。JDWP 用 `EventRequest.Set`（EXCEPTION，`ExceptionOnly` 修饰符）；类还没加载时等它的 ClassPrepare 再设。停下时 Debug 窗口标题旁显示 “Exception 'java.lang.IllegalStateException' occurred”（未被捕获时 “was not caught”），控制台也记一行；照 IntelliJ 的 `JavaStackFrame.createExceptionNodes`，顶层帧的 Variables 第一项是抛出的异常 `Exception = {IllegalStateException@408} "java.lang.IllegalStateException: boom"`，可展开 | ✅ |
| 断点跟随编辑（2026-10-16）：增删行的编辑按前后文本的逐行 diff（`debugger::follow_lines`）移动断点：不变的行保持，被改或删的行移到改动起点，移走后在别处原样出现的行跟过去（Extract Method 后断点随代码进新方法，Ctrl+Z 后回原处），同一行重复的只留一个。实测：CalcTest 第 34 行断点在提取后到第 39 行（新方法里同一句），撤销后回到第 34 行 | ✅ |
| 表达式框的代码补全和多行代码片段模式、异常断点的类过滤和条件、按文件分组、Remove once hit、依赖断点 | ⬜ |

Linux 实测（Xvfb，calc 示例的 JUnit 测试）：Breakpoints 对话框 + 添加 `java.lang.IllegalStateException`，调试 `throwsError` 停在第 22 行的 `throw`，标题旁显示 “Exception 'java.lang.IllegalStateException' occurred”。暂停在 `sum += items.get(i);`，光标放在 `get` 上按 Alt+F8，表达式框预填 `items.get(i)`，回车得到 `result = {Integer@447} 3`；求值 `items` 后展开得到 `[0] = {Integer@448} 1` 等三项；`items.get(1) * 10 + sum` 得 20；`nope + 1` 显示红字错误；历史列出三条；Add to Watches 后 Debug 窗口的 Watches 多了 `items.get(i)`。Breakpoints 对话框里列出 Calc.java:4（Shift+点行号加的黄色日志断点）和 CalcTest.java:34；给 CalcTest.java:34 填条件 `i == 2` 后切到另一项即保存到 breakpoints.json，再调试时停在 `i = 2`；列表里取消 Calc.java:4 的勾，gutter 立即变成空心；删除 Calc.java:4 后只剩一项；双击 CalcTest.java:34 打开该文件并跳到第 34 行。

## 39. Shape / Selector / Layer List 等 drawable 预览（2026-10-16）

第 32 节的 vector 预览扩展到 Android Studio 能预览的其他 drawable XML，同样是 Code / Split / Design 三种视图，编辑时重画；代码里和布局里引用它们的行（`@drawable/bg_button`、`R.drawable.badge`）在 gutter 显示缩略图。代码：`src/domain/drawable.rs`（按 `GradientDrawable`、`StateListDrawable`、`LayerDrawable` 的规则转成 SVG），`src/ui/file_editor/vector_preview.rs`、`src/ui/file_editor/previews.rs`。

| 功能 | 状态 |
|---|---|
| `<shape>`：rectangle（`<corners>` 的统一圆角或四角各自的半径，按 `min(w, h) / 2` 截断）、oval、line（只画描边，横穿中线）、ring（`innerRadius` / `thickness` 或按宽度的 `innerRadiusRatio` 9、`thicknessRatio` 3）；`<solid>` 或 `<gradient>`（linear 按 `angle` 的 45° 倍数，radial 带 `centerX/Y` 和 `gradientRadius`（dp 或 `50%`），`centerColor`）；`<stroke>`（宽度、颜色、`dashWidth` / `dashGap` 虚线），和 GradientDrawable 一样把形状内缩半个描边宽；`<size>` 为固有尺寸，没有时按 100 × 100 dp 画 | ✅ |
| `<selector>`：画默认状态下会选中的那一项（View 默认是 enabled + window_focused，其他状态为 false，取第一个匹配项），预览下面列出每一项的缩略图和状态（“pressed”、“not enabled”、“default”）；`<level-list>` 同样取匹配项 | ✅ |
| `<layer-list>`、`<ripple>`（不画 `@android:id/mask` 那一层）、`<transition>`：各层按 `left` / `top` / `right` / `bottom` 内缩，带 `width` / `height` 的层按 `gravity` 摆放（默认居中）；`<inset>`（`inset` 和四边）；`<color>` | ✅ |
| 引用：`android:drawable="@drawable/…"` 读取并画出所引用的 drawable XML（vector 经第 32 节的转换嵌入，id 改成唯一的），`@color/…` 和 `@dimen/…` 从项目资源解析；互相引用成环时报错 | ✅ |
| 位图（`<bitmap>`、PNG 引用、`.9.png`）在组合 drawable 里不画；sweep 渐变按 linear 画（SVG 没有锥形渐变）；`<animated-vector>`、`<animated-selector>`、`<clip>`、`<scale>`、`<rotate>`；按主题解析 `?attr/` | ⬜ |

Linux 实测（Xvfb，trans 示例）：`bg_button.xml`（pressed 引用渐变 shape、not enabled 是内联灰色 shape、默认引用 `@color/accent` 填充加 2dp 绿色描边的圆角 shape）的 Split 视图画出默认状态的 120 × 48 dp 圆角按钮，下面三个缩略图依次为橙黄渐变、灰色、默认；`badge.xml`（径向渐变圆、内缩 16dp 的 `@drawable/ic_star` vector、带虚线描边的 ring）画成 64 × 64 dp 的徽章；`layout/main.xml` 里 `android:background="@drawable/bg_button"` 和 `android:foreground="@drawable/badge"` 两行的 gutter 显示这两个 drawable 的缩略图。修正过一处：嵌入 vector 时 `preserveAspectRatio` 重复导致整张 SVG 无法解析，测试现在检查每张输出都是合法 XML。

## 40. Extract Method / Extract Function（Ctrl+Alt+M，2026-10-16）

照 IntelliJ 的 `ExtractMethodProcessor`（Java）和 Kotlin 的 Extract Function：选中的语句或表达式变成紧跟在所在方法之后的新私有方法，原处换成调用。代码：`src/index/extract_method.rs`（tree-sitter 分析）、`src/domain/extract_method.rs`（生成代码）、`src/ui/extract_method_dialog.rs`；编辑器右键 Refactor 菜单里也有。

| 功能 | 状态 |
|---|---|
| 选区：同一代码块里的若干完整语句（前后空白不计），或一个完整表达式；只选了半条语句时提示 “Selected block should represent a set of statements or an expression” | ✅ |
| 参数：选区里读到的、在选区之前声明的局部变量和参数，按首次出现的顺序；类型取声明处写的（Kotlin `n: Int`、Java `java.util.List<Integer> items`），Kotlin 未写类型时按初始化表达式推断（字面量、比较 / 逻辑运算为 Boolean、`Point(…)` 构造、算术里已知类型的变量、`for (i in 0..n)` 为 Int），推断不出时留空让用户在对话框填 | ✅ |
| 结果：选区里声明、选区之后还用到的变量（调用处 `val b = extracted(n)` / `long b = extracted(n);`，新方法末尾 `return b`），或选区之前声明、在选区里赋值且之后还用到的变量（`s = add(s)`；Kotlin 参数是 val，新函数开头先 `var s = s`）；超过一个时提示 “There are multiple output values for the selected code fragment” | ✅ |
| 跳转：选区以方法的最后一条 `return` 结束时调用处改成 `return extracted(…)`，新方法返回原方法的类型；中途的 return、目标循环在选区外的 break / continue 拒绝（lambda / 局部函数里的不算） | ✅ |
| 表达式：Kotlin 生成表达式体 `private fun extracted(x: Int) = x * 3 + 1`（类型由编译器推断），Java 生成 `return …;`，类型看不出时对话框要求填 Return type | ✅ |
| 对话框：Name（默认 “extracted”，被占用时 extracted1…）、Visibility（Kotlin private / internal / protected / public，Java 另有 package-private）、Java 的 Declare static（所在方法是 static 时固定勾选）、参数的名字和类型可改（改名会同步改新方法体里的用法）、Signature Preview；名字非法、参数重名或缺类型时显示原因且不能确定 | ✅ |
| 一次编辑完成，Ctrl+Z 一步撤销；完成后选中新方法名 | ✅ |
| 选区里的重复代码片段一并替换、参数顺序调整 / 去掉参数、Extract Parameter / Change Signature、Kotlin 的 receiver 和泛型参数、按语言服务器取类型 | ⬜ |

Linux 实测（Xvfb）：calc 的 `CalcTest.totals()` 选中 `int sum = 0;` 到循环结束，Ctrl+Alt+M 弹出 Extract Method：参数 `items`（`java.util.List<Integer>`），预览 `private int extracted(java.util.List<Integer> items)`；改名 `sumOf` 后原处变成 `int sum = sumOf(items);`，后面是带 `return sum;` 的新方法；Ctrl+Z 一步还原。ktjvm 示例 `Shop.check` 的两条语句提取为 `private fun report(n: Int)`，原处是 `report(n)`。

## 41. Merged Manifest（2026-10-16）

照 Android Studio 的清单编辑器：`AndroidManifest.xml` 下方有 “Text | Merged Manifest” 两个标签，Merged Manifest 是只读的合并结果，每一行按来源文件着色，右边是 “Manifest Sources”（来源文件和颜色）和 “Merging Errors”；随编辑实时重算（当前文件用编辑中的文本，其他读磁盘）。合并规则照 Gradle 插件的 ManifestMerger2。代码：`src/domain/manifest_merge.rs`（合并、注入、打印）、`src/usecase/merged_manifest.rs`（找来源、读 build 文件）、`src/ui/file_editor/merged_manifest.rs`。

| 功能 | 状态 |
|---|---|
| 来源和优先级：构建类型的覆盖清单 `src/debug/AndroidManifest.xml`（打开的是其他源集时用那个）高于 `src/main`，再低是 build 文件里 `project(":lib")` 依赖的库模块的清单（逐层传递，去重）；以 main 的结构为底，覆盖清单并入，库新增的元素追加在后 | ✅ |
| 元素按标签 + `android:name` 匹配（intent-filter 按内容），属性高优先级的赢；值不同且没有 `tools:replace` 时报 AS 原文格式的冲突 “Attribute application@android:theme value=(…) from … is also present at … value=(…). Suggestion: add 'tools:replace=…'”；`tools:node="remove" / "removeAll" / "replace"`、`tools:remove`；输出去掉 `tools:` 属性 | ✅ |
| build 文件注入（Kotlin DSL 和 Groovy）：`applicationId`（或 `namespace`）为 `package`，`versionCode` / `versionName`，`minSdk` / `targetSdk` 写进（必要时新建的）`<uses-sdk>`；`manifestPlaceholders`（`["k"] = "v"`、`mapOf("k" to "v")`、`[k: "v"]`）和 `${applicationId}` 替换 | ✅ |
| 打印成 AS 合并视图的样式：四空格缩进，多个属性时每个属性一行 | ✅ |
| 双击合并视图的一行打开它的来源文件并跳到对应行（元素和属性各自记住在来源里的行；build 文件注入的行打开 build 文件）。实测：双击 lib 颜色的 ScanActivity 行，打开 `lib/src/main/AndroidManifest.xml` 第 7 行 | ✅ |
| 远程依赖（AAR）的清单、产品风味 / 当前所选构建变体、隐式权限、`tools:overrideLibrary`、`tools:selector`、合并日志（Merging Log）里每个元素的来源说明 | ⬜ |

Linux 实测（Xvfb，示例工程 app + lib）：app 的 main 清单有 INTERNET 权限、MainActivity、`${mapsKey}` 的 meta-data 和标 `tools:node="remove"` 的 lib provider，debug 覆盖清单改 label（带 `tools:replace`）并加 `usesCleartextTraffic`，lib 有 CAMERA 权限、ScanActivity、provider 和自己的主题。Merged Manifest 依次显示：build.gradle.kts 着色的 `package="com.example.shop"`、versionCode 12 / versionName 2.1 和 `<uses-sdk minSdkVersion="24" targetSdkVersion="35">`，main 的 INTERNET 和 application（其中 `android:label="Shop (debug)"` 和 `usesCleartextTraffic` 两行是 debug 的颜色），`android:value="AIza-demo"`，lib 颜色的 ScanActivity、CAMERA 和 uses-feature，provider 已被移除；Merging Errors 列出 application 的 theme 在 app 和 lib 之间冲突。

## 42. Introduce Parameter（Ctrl+Alt+P，2026-10-16）

照 IntelliJ 的 Refactor › Introduce Parameter（Kotlin 的 `KotlinIntroduceParameterHandler`、Java 的 `IntroduceParameterHandler`）：选中函数里的一个表达式（不选时取光标处最内层的表达式），它变成函数新的最后一个参数，函数里的这处（或全部相同的几处）换成参数名，项目里对这个函数的调用都补上实参。代码：`src/domain/introduce_parameter.rs`（编辑和每个调用的实参）、`src/index/introduce_parameter.rs`（语法树分析、找调用）、`src/ui/introduce_parameter_dialog.rs`。

| 功能 | 状态 |
|---|---|
| 分析：表达式所在的函数、它的参数；表达式里用到的函数参数和类成员（字段、方法）；用到函数里的局部变量时拒绝（“Cannot introduce parameter: the expression uses local variable '…'”）；不在函数里时提示 | ✅ |
| 对话框：Name（按表达式建议，下拉可选其他建议，和已有参数 / 局部变量重名时报错）、Type（看得出时预填，否则要填）、“Replace all N occurrences”（函数里有多处相同表达式时）、Kotlin 的 “Introduce default value”；底部说明会改几处调用 | ✅ |
| 调用处：实参是表达式本身，其中用到的参数换成这次调用传的实参（复杂实参加括号），类成员前加上调用的接收者（`cart.total(10.0, 3)` → `cart.total(10.0, 3, 10.0 * cart.rate)`）；用了命名实参或省略了默认参数的 Kotlin 调用补命名实参 `amount = …` | ✅ |
| Kotlin 默认值：参数写成 `amount: Double = price * rate`，调用处都不改 | ✅ |
| 范围：当前文件（编辑器里一步撤销）和项目里其他含 `name(` 的 .kt / .java 文件（直接写盘并刷新） | ✅ |
| 按名字和实参个数匹配调用（同名的其他函数也可能被改），Java 的 Declare final / 通过重载委托 / 字段改 getter、Kotlin 扩展函数的接收者、Change Signature | ⬜ |

Linux 实测（Xvfb）：Kotlin 工程里 `Cart.total(price: Double, count: Int)` 的 `return price * rate * count + price * rate`，选中第一处 `price * rate` 按 Ctrl+Alt+P，对话框显示 “Replace all 2 occurrences”、“3 calls of total() get the new argument”；名字 amount、类型 Double，Refactor 后签名是 `total(price: Double, count: Int, amount: Double)`，函数体是 `return amount * count + amount`，同文件的 `total(1.0, 2)` 变成 `total(1.0, 2, 1.0 * rate)`，Main.kt 的 `cart.total(10.0, 3)` 和 `cart.total(count = 1, price = 2.0)` 分别变成 `cart.total(10.0, 3, 10.0 * cart.rate)` 和 `cart.total(count = 1, price = 2.0, amount = 2.0 * cart.rate)`。

## 43. 方法断点（2026-10-16）

照 IntelliJ 的 Java Method Breakpoint / Kotlin Function Breakpoint（默认的 emulated 方式，`JavaMethodBreakpoint` 不用很慢的 MethodEntry 事件，而是在方法的第一条代码上设断点）：在方法声明行点行号（或按 Ctrl+F8），如果这一行上没有方法体的代码，就设方法断点而不是行断点，gutter 上画成菱形。代码：`src/index/method_breakpoint.rs`（哪一行声明了哪个方法）、`domain::debugger::method_entries`、`usecase::debug_session` 的 `LineBreakpoint.method`。

| 功能 | 状态 |
|---|---|
| 声明行判断：Kotlin `fun` / Java 方法、构造方法（`<init>`）的名字在这一行，方法体是 `{…}` 块且第一条代码在后面的行；表达式体 `fun f() = …` 和单行方法 `{ return a + b; }` 仍是行断点 | ✅ |
| 设在哪：类加载后，类里同名方法中代码落在声明范围内的那个（区分重载、跳过 Kotlin 的 `name$default` 桥方法，认 internal 函数的 `name$module`），断点设在它第一条代码上；条件、Suspend、日志选项和行断点一样 | ✅ |
| 图标：菱形，已验证带勾、禁用为空心、不挂起为黄色（IntelliJ 的 breakpointMethod 系列）；Breakpoints 对话框分为 “Java Method Breakpoints” / “Kotlin Function Breakpoints”，标题如 “CalcTest.java:30 totals()”；持久化在 breakpoints.json 的 `method` 字段 | ✅ |
| Method exit（2026-10-20）：Breakpoints 对话框里方法断点的 Watch 选项 “Method entry”（默认开）/ “Method exit”（默认关），照 IntelliJ；退出用 JDWP 的 MethodExit 事件（`ClassOnly` 过滤到该类，事件来自别的方法时直接继续），停在方法返回处（`return` 那一行，局部变量还在）；JVM 和 ART 都支持。存为 breakpoints.json 的 `method_entry` / `method_exit`，改动后在运行中的会话里重设 | ✅ |
| 接口 / 抽象方法上的断点（停在所有实现里）、按类名 + 方法名的通配方法断点 | ⬜ |

Method exit 实测（Xvfb，JUnit 4 示例）：`Calc.add` 上的方法断点只勾 Method exit，Debug 'CalcTest.adds' 后停在 `add:6`（`return sum + 10;`），Variables 显示 `a = 1`、`b = 2`、`sum = 3`；F9 后第二次调用再停一次（帧 `adds:10`），再 F9 测试结束。

Linux 实测（Xvfb，calc 示例）：在 `CalcTest.java` 第 30 行 `public void totals() {` 按 Ctrl+F8，gutter 出现菱形；Debug 'totals'（Ctrl+Shift+F9）后菱形带勾，程序停在 `totals:31`（方法第一行代码）；Ctrl+Shift+F8 的 Breakpoints 对话框列在 “Java Method Breakpoints” 下。

## 44. 字段监视点（Field Watchpoint，2026-10-16）

照 IntelliJ 的 Java Field Watchpoint（`FieldBreakpoint`，JDWP 的 FieldModification / FieldAccess 事件加 FieldOnly 过滤）：在字段声明行点行号（或 Ctrl+F8）设字段监视点，gutter 上是眼睛图标；字段被写（默认）或被读时停在写 / 读它的那行。代码：`index::method_breakpoint::field_at`、`domain::debugger::FieldWatch`、`infra::jdwp_client` 的 `set_field_watch`、`usecase::debug_session`。

| 功能 | 状态 |
|---|---|
| 声明行判断：Java 的字段（`field_declaration`），Kotlin 类体或文件顶层的属性（它的 backing field 同名，顶层属性在 `FileKt` 类里）；函数里的局部变量不算 | ✅ |
| 类加载后按名字找到这个类自己的字段，设修改监视和 / 或读取监视；静态初始化里的赋值也会停（`<clinit>`），和 IntelliJ 一样 | ✅ |
| Breakpoints 对话框：“Java Field Watchpoints” / “Kotlin Field Watchpoints” 分组，标题如 “Calc.java:4 calls”，属性里多出 Watch: Field access / Field modification（会话中改动会重新设置）；条件、Suspend、日志选项照常；持久化在 breakpoints.json 的 `field`、`field_access`、`field_modification` | ✅ |
| 图标：IntelliJ 的 breakpointField 系列（已验证带勾、禁用、不挂起为黄色） | ✅ |
| 命中时在控制台写出旧值 → 新值、实例过滤（Instance filters）、只监视某个对象、没有 backing field 的 Kotlin 属性（自定义 getter） | ⬜ |

Linux 实测（Xvfb，calc 示例临时加 `static int calls = 0;` 和 `add` 里的 `calls++`）：在 `Calc.java` 第 4 行按 Ctrl+F8 出现眼睛图标；调试 `CalcTest.adds` 后图标带勾，先停在 `<clinit>:4`（静态初始化赋值），F9 后停在 `add:5`，Variables 显示 `a = 2`、`b = 2`；Breakpoints 对话框列在 “Java Field Watchpoints” 下，Field modification 默认勾选。

## 45. Mute Breakpoints（2026-10-16）

照 IntelliJ Debug 工具栏的 Mute Breakpoints（`XDebugSession.setBreakpointMuted`）：开关打开后，所有断点（行、方法、字段监视点、异常断点）都保留，但命中时不停也不打日志，程序直接继续；Run to Cursor 和单步照常。代码：`DebugSession::set_muted`（会话里命中的事件直接 resume）、`DebugService::toggle_mute`、Debug 工具栏的按钮。

| 功能 | 状态 |
|---|---|
| Debug 工具栏的 Mute Breakpoints 开关（按下状态），调试中切换立即生效，之后新开的调试会话沿用 | ✅ |
| 静音时 gutter 上启用的断点画成灰色（IntelliJ 的 breakpointMuted / breakpointMethodMuted / breakpointFieldMuted），行不再加红底；禁用的仍是空心 | ✅ |
| Run 菜单里的 Mute Breakpoints 勾选项（和工具栏按钮同步）；静音状态跨重启保留（设置目录下的 `breakpoints-muted`；断点本身是全局保存的，静音也跟着全局，不按项目分） | ✅ |

Linux 实测（Xvfb，calc 示例）：在 `totals()` 循环体第 34 行设断点，调试后在 `i = 0` 停下；点 Mute Breakpoints，断点变灰，按 F9 后剩下两次循环不再停，测试直接跑完通过。

## 46. 控制台：Build Output / Run / Debug Console（2026-10-16）

照 IntelliJ 的 `ConsoleViewImpl`：控制台是一个只读编辑器，而不是一行行独立的文字。代码：`src/ui/console_view.rs`（控制台组件）、`src/domain/console_links.rs`（超链接识别，对应 IntelliJ 的 `UrlFilter` 和文件路径过滤器）。

| 功能 | 状态 |
|---|---|
| 鼠标拖动跨行连续选中（和编辑器一样，行与行之间没有间隔），双击选词，Ctrl+A 全选、Ctrl+C 复制，Ctrl+F 在输出里查找 | ✅ |
| 超链接：网址（`https://docs.gradle.org/…`，点击用浏览器打开）；源码路径带行列（`/src/Foo.kt:12:5`、Windows 的 `file:///C:/…/Foo.kt:12:5`、`Foo.kt: (12, 5)`、javac 的 `Foo.java:7: error`，以及 html / txt 报告），点击打开文件并跳到对应行；拖选时不算点击 | ✅ |
| 颜色：Gradle 的失败行（`FAILURE`、`e: `、`: error:`、`FAILED`）为红色，Run 控制台的命令为灰色、错误为红色 | ✅ |
| 跟随末尾（照 `ConsoleViewImpl.isStickingToEnd`）：视图在最底部时新输出自动跟随；往上滚了就停在原处，滚回最底部或点 Scroll to the End 恢复跟随；Logcat 从开头丢弃旧行时视图停在同样的行上 | ✅ |
| 用在 Build Output 的右侧、Run 的 app 控制台、Debug 的 Console 标签 | ✅ |
| Logcat 的日志也在这种控制台里（Android Studio 的 Logcat 同样是编辑器）：拖选跨行复制、Ctrl+F 查找；列和以前一样（时间、pid-tid、按哈希着色的 tag、包名、带底色的等级标签、按等级着色的消息），网址和堆栈帧的 `Foo.kt:42` 是链接（点击打开项目里的源文件并跳到那一行）；新日志追加在末尾，缓冲区满时从开头丢弃旧行，而不是整体重绘；改过滤器、Standard / Compact、Clear 时整体重建；Soft-Wrap、Scroll to the End 按钮照常 | ✅ |
| 控制台左侧的竖工具条（照 AS 的 Run / Build 控制台）：Soft-Wrap（开关）、Scroll to the End、Clear All（Build Output 清掉输出文字、问题列表保留；Run 清掉 app 控制台）；Debug 的 Console 只有前两个 | ✅ |
| ANSI 转义码（照 `AnsiEscapeDecoder`）：SGR 的 16 色（Darcula / Light 两套配色）、256 色、24 位真彩色，粗体、暗淡、斜体、下划线、反色，样式跨行延续；其他控制序列（光标移动、清行、OSC 标题）去掉；`\r` 回车覆盖本行（进度条只留最后一次）。代码：`src/domain/ansi.rs` | ✅ |
| 输出上限（照 IntelliJ 的 cycle buffer，`idea.cycle.buffer.size` 默认 1024 KB）：Build / Run / Debug 控制台超过 1 MB 时从开头丢弃最旧的行（Logcat 有自己的缓冲区） | ✅ |
| Print、OSC 8 超链接、可设置的缓冲区大小 | ⬜ |

同时修复：未同步过的 Gradle 工程，根目录 `build.gradle(.kts)` 里写了 `alias(libs.plugins.android.application) apply false`（只声明插件、不应用），会被误判成 Application 模块。运行配置因此变成以根工程命名（如 “mytv-android”），构建任务成了 `:assembleDebug`，Gradle 报 “Cannot locate tasks that match ':assembleDebug'”。现在判断模块类型时会忽略带 `apply false` / `apply(false)` 的行，运行配置是 `app`，任务是 `:app:assembleDebug`。

Linux 实测（Xvfb，calc 示例，故意删掉一个分号）：Ctrl+F9 后 Build Output 右侧显示 Gradle 输出，失败行为红色，`Calc.java:4` 路径和 `file:///…/problems-report.html` 带下划线；点击 `Calc.java:4` 链接打开 `Calc.java` 并定位到第 4 行；跨 4 行拖选后 Ctrl+C，粘贴到 Commit Message 得到完整的多行文本。（Xvfb 没有窗口管理器，窗口不会被激活，编辑器不画选区高亮，这一点只能在桌面上看。）calc 构建成功后，Build Output 控制台左侧有 Soft-Wrap / Scroll to the End / Clear All 三个按钮，点 Clear All 输出清空，左边的构建状态保留。

## 47. 没有源码的库：反编译存根（2026-10-16）

照 IntelliJ 打开没有源码的库时的 “Decompiled .class file”：Gradle 缓存里某个依赖没有 `-sources.jar` 时，读取它的 class 文件（`.aar` 里的 `classes.jar`，或者 `.jar`），生成方法体为 `{ /* compiled code */ }` 的存根源码，和源码一样建索引，Ctrl+点击、Go to Class、Find Usages 都能走到它（只读）。代码：`src/domain/class_stub.rs`（class 文件解析和存根生成）、`index::libraries::decompile_stubs`（按归档的大小和修改时间缓存到 `decompiled/` 下）。

| 功能 | 状态 |
|---|---|
| 解析 class 文件：常量池、类名、父类和接口、public / protected 的字段和方法（跳过 synthetic、bridge 和 Kotlin 生成的 `$default`、`access$` 方法） | ✅ |
| Kotlin 编译的类（带 `kotlin.Metadata`）生成 `.kt` 存根：文件门面类 `BoxKt` 变成顶层函数，`getX()` / `isX()` 变成 `val` / `var` 属性，带 `INSTANCE` 的类是 `object`，`$Companion` 是 `companion object`，以及 interface / enum class / annotation class / abstract / open；类型映射成 Kotlin 名字（`Int`、`String`、`IntArray`、`Array<T>`、`Function1<*>`） | ✅ |
| 其他类生成 `.java` 存根：修饰符、extends / implements、字段、构造方法、方法（varargs 为 `T...`），嵌套类写在外层类里 | ✅ |
| class 文件里没有参数名，参数写成 `p0`、`p1`…；Kotlin 的 `@InlineOnly` 函数（如 `println`）在字节码里是 private 的，存根里没有（IntelliJ 靠解析 Kotlin 元数据才能显示） | ⬜ |
| 编辑器顶部的提示条 “Decompiled .class file: no sources were found for group:artifact:version” 和 **Download Sources**：依次从 Google Maven、Maven Central 下载 `-sources.jar`，存进 Gradle 缓存里这个版本的目录，重新扫描库，源码解压出来后打开这个类的源码并关掉存根标签页（同 IntelliJ 用源码替换反编译视图）；下载失败时在提示条上写出原因 | ✅ |
| 解析 Kotlin 元数据得到真实的参数名、可空类型和泛型；Signature 属性里的 Java 泛型；一次下载所有库的源码（Gradle 的 “Download Sources” 任务）；打开源码后停在存根里光标所在的同一个成员上 | ⬜ |

Linux 实测（Xvfb）：Kotlin 工程依赖 `com.google.code.gson:gson:2.11.0`（Gradle 缓存里只有 jar，没有源码包），`import com.google.gson.Gson` 后按住 Ctrl 点 `Gson()`，打开只读的 `Gson.java` 存根（面包屑显示 `Gradle: com.google.code.gson:gson:2.11.0`），光标停在构造方法 `public Gson() { /* compiled code */ }`，下面列出 `toJson(Object p0)` 等方法；顶部提示条点 Download Sources，`gson-2.11.0-sources.jar` 下载到 Gradle 缓存，约 20 秒后标签页换成带版权头和 import 的真正 `Gson.java`。

## 48. Change Signature（Ctrl+F6，2026-10-16）

照 IntelliJ 的 Refactor › Change Signature（Kotlin 的 `KotlinChangeSignatureHandler`、Java 的 `ChangeSignatureHandler`）：光标在函数名或函数体里按 Ctrl+F6（macOS ⌘F6），对话框里改名字、返回类型和参数（改名、改类型、增删、上下移动），函数声明、函数体里改名参数的用法以及项目里的调用一起改。代码：`src/domain/change_signature.rs`（声明、用法和每个调用的编辑）、`src/index/change_signature.rs`（语法树分析）、`src/ui/change_signature_dialog.rs`。

| 功能 | 状态 |
|---|---|
| 分析：光标所在的最内层函数（Kotlin `fun`、Java 方法和构造方法），每个参数原样的文本、修饰符（`vararg`、`final`、注解）、类型、Kotlin 默认值，以及它在函数体里的用法（按作用域，同名的局部变量不算） | ✅ |
| 对话框：Name、Return type（Kotlin 留空即去掉返回类型）、参数表（Name / Type / 新增参数的 Default value (for calls)），+ − ↑ ↓ 按钮，Signature Preview，将更新的调用数；名字非法、参数重名、缺类型、新参数缺默认值时显示原因（错误行始终占位，对话框不跳动）；新增参数时焦点放到新行 | ✅ |
| 声明：没改的参数保持原文（修饰符、默认值、注解都在），改了的按语言重写；参数原本每行一个时保持每行一个；返回类型增删改 | ✅ |
| 调用：按新顺序排列实参，删掉的参数去掉，新参数填默认值；Kotlin 调用里原本用命名实参、或者顺序变了又有命名实参、或者省略了有默认值的参数时，后面的实参写成命名实参；函数改名时调用处一起改；其他 .kt / .java 文件里的调用直接写盘并刷新 | ✅ |
| 按名字和参数个数匹配调用（同名同参数个数的其他函数也会被改），Java 的 `new A(…)` 构造调用、方法引用 `::name`、Kotlin 的尾随 lambda、可见性、异常列表、覆盖方法一起改、委托旧签名 | ⬜ |

Linux 实测（Xvfb）：Kotlin 工程里 `Cart.total(price: Double, count: Int): Double` 按 Ctrl+F6，改名 `sum`，把 `count` 上移，新增 `tax: Double`（调用处默认值 `0.0`），Refactor 后声明是 `fun sum(count: Int, price: Double, tax: Double): Double`，同文件的 `total(1.0, 2)` 变成 `sum(2, 1.0, 0.0)`，Main.kt 的 `cart.total(10.0, 3)` 和 `cart.total(count = 1, price = 2.0)` 变成 `cart.sum(3, 10.0, 0.0)` 和 `cart.sum(count = 1, price = 2.0, tax = 0.0)`。

## 49. 基本编辑操作：复制行、注释、移动行、扩大选区…（2026-10-17）

照 IntelliJ 默认键位（Default / macOS keymap）补上编辑器里最常用的一组操作，每个都是一次可撤销的修改。代码：`src/domain/line_edit.rs`（纯文本 + 选区的编辑）、`src/index/selection.rs`（按语法树扩大选区）、`src/ui/file_editor/editing.rs`。

| 操作 | 键 | 状态 |
|---|---|---|
| Duplicate Line or Selection：没选中时复制当前行到下方（光标跟到新行），选中时在后面复制一份并选中它 | Ctrl+D / ⌘D | ✅ |
| Comment with Line Comment：选中的各行（或当前行）在第一列加 `//`（Python、YAML、properties 用 `#`，SQL 用 `--`；XML 每行包成 `<!-- … -->`），全部已注释时去掉；没选中时光标移到下一行 | Ctrl+/ / ⌘/ | ✅ |
| Comment with Block Comment：选区包成 `/* … */`（XML `<!-- … -->`），已是块注释时去掉；没选中时插入空注释、光标在中间 | Ctrl+Shift+/ / ⌥⌘/ | ✅ |
| Move Line Up / Down：选中的行（或当前行）和上 / 下一行交换，选区跟着走 | Alt+Shift+↑/↓ | ✅ |
| Extend Selection：单词 → 外层语法节点逐级扩大，括号 / 花括号先选内部内容再选整体，最后是全文；没有语法时按行、全文 | Ctrl+W / ⌥↑ | ✅ |
| Shrink Selection：按扩大的步骤退回 | Ctrl+Shift+W / ⌥↓ | ✅ |
| Join Lines：下一行（或选中的行）接到当前行末尾，去掉缩进、用一个空格连接（括号边不加空格） | Ctrl+Shift+J | ✅ |
| Start New Line：在当前行下面开新行并缩进（行末是 `{` 时多缩进一级） | Shift+Enter | ✅ |
| Toggle Case：选区（或光标处的单词）转大写，已全大写时转小写 | Ctrl+Shift+U / ⌘⇧U | ✅ |
| Move Statement Up / Down：光标所在行开始的整个语句 / 声明（多行的函数、类一起）和同一块里的上 / 下一个交换，中间的空行留在原处；到块的边上不动；没有语法的文件按行移动（`src/index/selection.rs::move_statement`） | Ctrl+Shift+↑/↓ / ⌘⇧↑/↓ | ✅ |
| Move Element Left / Right：光标所在的参数、实参、列表项和同一个逗号分隔列表里的前 / 后一个交换（Kotlin 参数连默认值一起），光标跟着走；嵌套时取最里层的列表（`src/index/selection.rs::move_element`） | Ctrl+Alt+Shift+←/→ / ⌥⌘⇧←/→ | ✅ |
| Code › Folding：Collapse（光标处最里层的可折叠区域，再按折外面一层，光标移到折叠的首行）/ Expand（光标处折起的区域）；Collapse All / Expand All（vendor 的编辑器补了 `fold_at_line` / `fold_all`，标 `Junction:`） | Ctrl+- / Ctrl+=，Ctrl+Shift+- / Ctrl+Shift+=（⌘ 同理） | ✅ |
| 多光标下的这些操作、注释时按代码风格设置决定 `//` 是否加在缩进后、Move Statement 移进 / 移出块 | | ⬜ |

Linux 实测（Xvfb，Kotlin 文件）：光标在 `return price * rate …` 行，Ctrl+D 复制出第 6 行，Ctrl+/ 把它注释成 `//        return …` 并把光标移到 `}`，Alt+Shift+↑ 把 `}` 移到注释行上面；三次 Ctrl+Z 逐个撤销。Shapes.kt 里光标在 `class Tile : Square()` 上按 Ctrl+Shift+↑，它整个跳到四行的 `open class Square … { … }` 前面；再按两次 Ctrl+Shift+↓，依次越过 Square 和 `class Disc`。Main.kt 里光标在 `cart.total(count = 1, price = 2.0)` 的 `count` 上按 Ctrl+Alt+Shift+→，变成 `total(price = 2.0, count = 1)`。Cart.kt 光标在 `return` 行：Ctrl+- 折起 `total`，再按折起整个类，Ctrl+= 展开类（`total` 仍折着）；Ctrl+Shift+- 全部折起，Ctrl+Shift+= 全部展开。光标在 `rate` 上连按三次 Ctrl+W，选区依次是 `rate`、`price * rate`、`price * rate * count`。

## 50. Call Hierarchy（Ctrl+Alt+H，2026-10-17）

照 IntelliJ 的 Hierarchy 工具窗口（右侧）做调用层级：光标所在的函数作根，下面是调用它的函数（"Callers of Cart.total"），每层点开再查它的调用者。代码：`src/index/call_hierarchy.rs`（调用点所在的函数 / 类 / 文件）、`src/ui/hierarchy_view.rs`。

| 功能 | 状态 |
|---|---|
| Ctrl+Alt+H / 右键菜单 Call Hierarchy：光标所在函数的调用者树（Kotlin、Java） | ✅ |
| 调用者按所在函数分组，显示为 `类.函数()`，多处调用显示 `(N usages)`；不在函数里的调用（字段初始化、顶层）归到类或文件 | ✅ |
| 逐层展开（按需查找），递归调用标 "recursive call" 不再展开；查不到调用者时去掉展开箭头 | ✅ |
| 双击打开调用位置（根节点打开声明） | ✅ |
| Callee Hierarchy：工具栏切换调用者 / 被调用者方向（图标同 IntelliJ，用 Supertypes / Subtypes）；被调用者是函数体里的调用（Kotlin 调用和构造、Java 方法调用和 `new`），解析到声明后按函数分组，同样逐层展开；双击打开被调用的声明；Ctrl+Alt+H 总是从调用者方向开始 | ✅ |
| 作用域过滤、找不到声明的调用（如没有索引的标准库）显示出来 | ⬜ |

Linux 实测（Xvfb，`ip` 工程）：光标在 `Cart.total` 上按 Ctrl+Alt+H，右侧显示 `Cart.total()` 下的 `Cart.use()` 和 `main() (2 usages) (Main.kt)`；展开 `Cart.use()` 没有调用者，箭头消失；双击 `main()` 打开 Main.kt 第 5 行 18 列的 `total`。在 Main.kt 的 `main` 上 Ctrl+Alt+H 再点 Callee Hierarchy，显示 `Cart() (Cart.kt)` 和 `Cart.total() (2 usages)`（第一次要等 kotlin-lsp 热身约 10 秒，之后不到 1 秒）；双击 Cart.total 打开 Cart.kt 第 4 行 9 列。

## 51. Type Hierarchy（Ctrl+H，2026-10-17）

和 Call Hierarchy 共用 Hierarchy 工具窗口，照 IntelliJ 的三种视图（工具栏切换，图标取自 intellij-community `expui/hierarchy`）。光标在类型名上（声明或引用）时用这个类型，否则用光标所在的类。代码：`src/index/hierarchy.rs`（`type_hierarchy`）、`src/index/call_hierarchy.rs`（`type_at`）、`src/ui/hierarchy_view.rs`。

| 功能 | 状态 |
|---|---|
| Ctrl+H / 右键菜单 Type Hierarchy（编辑器输入框自己的 Ctrl+H 替换让给它，替换用 Ctrl+R） | ✅ |
| Type Hierarchy：父类链（跳过接口）从上到下排到当前类型，下面是它的全部子类型；打开时选中当前类型 | ✅ |
| Supertypes Hierarchy：当前类型继承 / 实现的全部类型，逐层向上；工程和库里找不到的类型灰色显示 | ✅ |
| Subtypes Hierarchy：继承当前类型的全部类型，逐层向下（工程的在前，库的在后） | ✅ |
| 类 / 抽象类 / 接口 / 枚举 / object 图标（Kotlin 带角标），名称后显示所在文件；双击打开声明，箭头折叠 / 展开 | ✅ |
| 类的父类链顶上是 `Any`（Kotlin）/ `Object`（Java），接口和枚举没有；库里索引到时可双击打开，否则灰色 | ✅ |
| 按包显示位置、作用域过滤 | ⬜ |

Linux 实测（Xvfb，`ip` 工程里的 Shapes.kt：`Shape` ← `Base` ← `Square` ← `Tile`，`Disc : Base()`）：光标在 `Square` 类里按 Ctrl+H，显示 `Base › Square › Tile` 并选中 Square；Supertypes 显示 `Square › Base › Shape` 和灰色的 `Comparable`；Subtypes 显示 `Square › Tile`，双击 Tile 打开第 15 行。

## 52. Method Hierarchy（Ctrl+Shift+H，2026-10-17）

同一个 Hierarchy 工具窗口。光标所在方法的类往上找到最上层声明这个方法的类型（接口也算），从它开始列出全部子类型，每个类型前面照 IntelliJ 的图例标记（图标取自 intellij-community `hierarchy/`）：

| 标记 | 含义 |
|---|---|
| 绿色 + | 这个类型定义了该方法；双击打开这个方法 |
| 红色 − | 没有定义，从父类继承 |
| 红色 ! | 没有定义但应该定义：上层是接口或抽象方法、到这里还没有实现、而这个类不是抽象的 |

代码：`src/index/hierarchy.rs`（`method_hierarchy`）、`src/ui/hierarchy_view.rs`。右键菜单也有 Method Hierarchy。

Linux 实测（Xvfb，Shapes.kt：`interface Shape { fun area() }`、`abstract class Base : Shape`、`Square : Base()` 实现 area、`Tile : Square()`、`Disc : Base()`）：光标在 `Square.area` 上按 Ctrl+Shift+H，显示 `+Shape › −Base › (!Disc, +Square › −Tile)`，选中 Square。

## 53. Recent Locations（Ctrl+Shift+E，2026-10-17）

照 IntelliJ 的 Recent Locations 弹窗：最近到过和改过的位置，新的在前，每项显示文件名、所在的函数或类（`Cart.total`）、位置前后几行代码（去掉公共缩进、带行号、语法着色）。代码：`src/ui/workspace/history.rs`（`RecentPlaces`）、`src/ui/recent_locations.rs`、`src/ui/workspace/navigation.rs`。

| 功能 | 状态 |
|---|---|
| 记录：跳转（Ctrl+B、Go to、Back / Forward、Ctrl+G）和切换文件时离开的位置，编辑时的位置（标 changed），打开弹窗时的当前位置；同一文件相距两行以内的合并成一个（保留 changed），最多 50 个 | ✅ |
| 弹窗：输入过滤（文件名、所在函数、代码）；↑/↓ 选择，Enter / 单击跳到该位置；默认选中当前位置的上一个 | ✅ |
| Show changed only 复选框，或在弹窗里再按 Ctrl+Shift+E 切换；标题随之变成 "Recently Changed Locations" | ✅ |
| Navigate 菜单项、Keymap 设置里的条目 | ✅ |
| Navigate › Last Edit Location（Ctrl+Shift+Backspace / ⌘⇧⌫）：跳到最近一次编辑的位置，在那里再按就往前一个编辑位置，到头提示 "No more edit locations" | ✅ |
| 重启后保留（随编辑器会话一起存，见 §56） | ✅ |
| 编辑时已记录位置的行号跟着移动：在它上面加了 / 删了几行，它就跟着下移 / 上移（按编辑前后的行数和光标所在行推算；IntelliJ 用 RangeMarker）；被删掉的行里的位置落到删除处 | ✅ |
| 删除某一项、多光标或替换整段文本时精确跟踪 | ⬜ |

Linux 实测（Xvfb，`ip` 工程）：在 Cart.kt 的 `use` 上 Ctrl+B 跳到 `total`，打开 Main.kt 在第 6 行末尾输入 ` // hi`，再打开 Shapes.kt；Ctrl+Shift+E 列出 Shapes.kt（Tile）、Main.kt（main，changed，三行代码含 `// hi`）、Cart.kt（Cart.total）、Cart.kt（Cart.use）；再按 Ctrl+Shift+E 只剩 Main.kt；输入 `cart` 过滤出含 cart 的三项；↓ Enter 打开 Cart.kt 第 4 行第 9 列。Last Edit Location：在 Cart.kt 第 8 行、Main.kt 第 6 行各改一处后切到 Shapes.kt，连按 Ctrl+Shift+Backspace 依次到 Main.kt 6:53、Cart.kt 8:35，第三次提示没有更多。

## 54. Inline Variable / Function（Ctrl+Alt+N，2026-10-17）

照 IntelliJ 的 Inline（局部变量）：光标在 Kotlin / Java 局部变量的声明或任一使用上按 Ctrl+Alt+N（右键 Refactor › Inline…），每个使用换成它的初始值，声明（独占的行整行）删掉，一次可撤销的修改，光标停在第一个被替换的位置。代码：`src/index/inline.rs`。

| 功能 | 状态 |
|---|---|
| 初始值是带运算符的表达式、而使用处外面还有运算时加括号（`(a - 1) * (a - 1)`）；作为参数、返回值、赋值右边时不加 | ✅ |
| Kotlin 字符串模板：`"$x"` 换成 `"${a + 1}"`（初始值只是一个名字时 `"$size"`） | ✅ |
| 拒绝并照 IntelliJ 的提示：不在变量名上、没有初始值、没有使用、声明后被赋值 / `++` | ✅ |
| 顺带修正：局部变量的解析（Ctrl+点击、Find Usages、Rename）现在认得 Kotlin 字符串模板里的 `$x`，以前 Rename 局部变量会漏改模板里的使用 | ✅ |
| Inline Function：光标在函数声明或本文件里的调用上（同一个 Ctrl+Alt+N，不是局部变量时）；函数体只有一个表达式（`= …` 或只有一条 `return …`）时，选 Inline All and Remove 后，工程里每个调用都换成函数体（参数换成实参，具名实参按名字对应；类成员加上调用方的接收者，`cart.total(…)` 里的 `rate` 变成 `cart.rate`；外面有运算时加括号），然后删掉声明。其他文件经 `usecase::update_calls` 改写。不内联：多条语句、递归、调用省略了有默认值的参数 | ✅ |
| 对话框照 IntelliJ："Function total() has 3 usages."，Inline All and Remove（默认）；在调用上执行时多一个 Inline This Only，只换这一处、保留函数 | ✅ |
| Inline Property / Field（同一个 Ctrl+Alt+N，光标在类 / object / companion 的属性、顶层属性、`const val` 或 Java 字段的名字或本文件的引用上）：对话框 "Property MAX has 2 usages."，Inline All and Remove 把工程里每个引用换成初始值（需要时加括号，模板里写成 `${10}`）、删掉不再需要的 import、删掉声明；在引用上执行时多一个 Inline This Only。代码：`src/index/inline_field.rs`、`usecase::project_edits` | ✅ |
| 没有类型解析，只跟随不会认错的引用：本文件里的裸名字和 `this.x`，任何文件里的 `Owner.x`（companion 的也算），导入了它的文件里的裸名字；遇到 `other.x`、`::x` 就停下并指出位置，不去猜；初始值里有名字时只在本文件内联（换个文件同名的可能是别的东西）；被赋值的 `var` / 非 final 字段拒绝 | ✅ |
| 顺带修正：Inline Variable 以前把 Kotlin 类 / object 里的属性也当成局部变量，只改本文件，其他文件里的 `Limits.MAX` 会断掉；现在只接受函数体里的局部变量 | ✅ |
| 预览、按类型解析 `other.x` | ⬜ |

Linux 实测（Xvfb，`ip` 工程 Main.kt）：光标在 `val cart = Cart(0.2)` 的 `cart` 上按 Ctrl+Alt+N，两处 `cart.total(…)` 变成 `Cart(0.2).total(…)`，声明行删除；再按一次弹出 "Cannot perform refactoring. Caret should be positioned at the name of element to be inlined."。Inline Function：Cart.kt 光标在 `total` 上按 Ctrl+Alt+N，确认 3 处后 `fun use() = 1.0 * rate * 2 + 1.0 * rate`，Main.kt 的两处变成 `10.0 * cart.rate * 3 + 10.0 * cart.rate` 和 `2.0 * cart.rate * 1 + 2.0 * cart.rate`（具名实参 `count = 1, price = 2.0` 对上了）。

## 55. Unwrap/Remove（Ctrl+Shift+Delete，2026-10-17）

照 IntelliJ 的 Code › Unwrap/Remove：光标处由内向外列出能拆的语句，弹窗标题 "Choose the statement to unwrap/remove"，选中后一次可撤销的修改，光标停在留下的第一条语句。代码：`src/index/unwrap.rs`，弹窗沿用 Alt+Enter 的列表。

| 选项 | 效果 | 状态 |
|---|---|---|
| Unwrap 'if...' | 光标在条件或 then 分支：then 分支的语句替换整个 `if`（去掉花括号、按原缩进） | ✅ |
| Unwrap 'else...' / Remove 'else...' | 光标在 else 分支：用 else 的语句替换 `if`，或只删掉 else 分支 | ✅ |
| Unwrap 'for...' / 'while...' / 'do...' | 循环体的语句替换循环 | ✅ |
| Unwrap 'try...' | try 块的语句替换 `try`，`finally` 的语句接在后面，catch 去掉 | ✅ |
| Kotlin 作为值的 `if`（`val x = if …`）不提供；右键菜单 Unwrap/Remove… | | ✅ |
| 选中某项时高亮会受影响的代码：整个语句，Remove 'else...' 只高亮要删的 else 部分 | | ✅ |
| Unwrap 'run {...}'（Kotlin 语句位置上只带一个 lambda 的调用：`run`、`apply`、`with` 外的 `x.let` 等）：lambda 体的语句替换调用 | lambda 的参数（`it`）不改写 | ✅ |
| Remove 'catch...' / Remove 'finally...'：光标在 catch / finally 里，只删掉这一段 | | ✅ |
| Unwrap 'when...' / 'switch...'：光标所在分支的语句替换整个 `when` / `switch`（Java 的 `case:` 分组去掉结尾的 `break`，`case ->` 取箭头后的语句或块） | | ✅ |
| 单独的花括号块、Remove 某个 when / case 分支 | | ⬜ |

Linux 实测（Xvfb，`ip` 工程 Flow.kt：`for` 里套 `if / else`）：光标在 else 分支的 `println("small $i")` 上按 Ctrl+Shift+Delete，弹出 Unwrap 'else...'、Remove 'else...'、Unwrap 'for...'；选 Remove 'else...' 后只剩 `if (i > 2) { println("big $i") }`；光标在 `for` 体末尾时只有 Unwrap 'for...'，选中后 `if / else` 提到函数体一层、缩进少一级。

## 56. 重新打开工程时恢复编辑器（2026-10-17）

照 IntelliJ 重新打开工程的样子：上次打开的标签页（按原顺序）、每个标签的光标位置、当时活动的标签，以及 Recent Locations 的位置列表，都在工程再打开时恢复。存于设置目录的 `editor-sessions.json`，按工程根目录分开。代码：`src/infra/editor_session_store.rs`、`src/ui/workspace/session.rs`。

| 功能 | 状态 |
|---|---|
| 保存时机：切换 / 打开 / 关闭标签、编辑后约 2 秒（合并连续的变化），以及退出（Exit）和关闭窗口时 | ✅ |
| 恢复：工程打开后一次；已删除的文件跳过；恢复不算一次访问，Recent Locations 的顺序不变 | ✅ |
| 固定（pin）的标签保持固定 | ✅ |
| 同一窗口里换到另一个工程（标题栏工程菜单的 Recent Projects）：先存好旧工程的会话、关掉它的标签，再恢复新工程的；以前旧工程的标签会留在新工程里 | ✅ |
| 启动时不带文件夹参数（开始菜单 / 快捷方式）：当前目录是仓库就开它（终端里运行），否则重新打开上一个工程，照 IntelliJ 默认的 Reopen projects on startup；以前会在欢迎页报 "Cannot open the folder … not inside a git repository" | ✅ |
| 开关：Settings › Appearance & Behavior › System Settings › Reopen projects on startup（默认开） | ✅ |
| System Settings › Confirm before exiting the IDE（默认开）：File › Exit 和关窗口时问 "Are you sure you want to exit Junction?"，勾 Do not ask again 会关掉这个设置；关窗口前先保存编辑器会话 | ✅ |
| File › Close Project：存好会话、关掉标签，回到欢迎页（标题栏显示 No Project / No branch）；在欢迎页再点这个工程，标签恢复 | ✅ |
| 分屏、不同 revision 的标签、工具窗口的布局 | ⬜ |

Linux 实测（Xvfb，`ip` 工程）：打开 Cart.kt 并把光标放在第 8 行，再打开 Main.kt；等 3 秒后直接结束进程再启动，两个标签按原顺序回来、Main.kt 是活动标签，点 Cart.kt 光标在 8:14。从工程菜单换到 calc：ip 的标签关掉，calc 上次打开的 Calc.class 回来；再换回 ip，Cart.kt、Main.kt 回来。

## 57. Safe Delete（Alt+Delete，2026-10-17）

照 IntelliJ 的 Safe Delete：光标在类、函数 / 方法、属性 / 字段或局部变量的声明名上按 Alt+Delete（macOS ⌘⌫，右键 Refactor › Safe Delete…）。先在后台查找用处（Find Usages 同一套索引，不算声明本身和它内部的递归调用）：

| 情况 | 行为 | 状态 |
|---|---|---|
| 没有用处 | "Delete function 'use'?" → Delete：删掉声明的整行（连同紧挨在上面的文档注释），并处理空行：不留两个空行，也不在 `}` 前留空行；一次可撤销 | ✅ |
| 有用处 | "Usages Detected：Function 'total' has 3 usages that are not safe to delete."，列出 `文件:行`，View Usages（默认，打开 Find 工具窗口）或 Delete Anyway | ✅ |
| 同时删掉调用 / 引用处、搜索注释和字符串、删文件 | | ⬜ |

代码：`src/index/safe_delete.rs`（声明和要删的行）、`domain::line_edit::declaration_lines`（删声明时的空行规则，Inline Function 也用它）。

Linux 实测（Xvfb，`ip` 工程 Cart.kt）：在 `total` 上按 Alt+Delete 弹出 Usages Detected，列出 Cart.kt:8、Main.kt:5、Main.kt:6；在 `use` 上按 Alt+Delete 弹出 "Delete function 'use'?"，Delete 后 `fun use()` 和它上面的空行一起删掉，类的 `}` 紧接在 `total` 后面。

## 58. 构建环境和缺 DLL 诊断（2026-10-17）

为了排查用户在 Windows 上编 APK 时弹出的“找不到 api-ms-win-eventing-consumer-l1-1-0.dll”：junction.exe、aapt2（8.7 / 8.13）、platform-tools、Temurin 17 / 21、JBR 21、Gradle 8.11 自带的原生库都查过导入表，没有一个依赖它，所以改成让程序自己报出是谁。

| 功能 | 状态 |
|---|---|
| Build Output 第二行写出 Gradle 用的 JDK（来源、厂商、版本、路径），Windows 上还有系统版本（`RtlGetVersion`，如 `Windows 10.0.19045`） | ✅ |
| Windows 上启动时 `SetErrorMode(SEM_FAILCRITICALERRORS)`，子进程继承：构建里缺 DLL 的程序不再弹系统框，而以 0xC0000135 退出 | ✅（Windows 未实测） |
| 输出里出现 0xC0000135 / -1073741515 时追加一行，说明是哪个程序缺 DLL（取 Gradle 的 `Process 'command '…''`） | ✅ |
| Build 窗口左侧列出 Gradle 的 “What went wrong”（Android Studio 的失败节点）：不是编译错误的失败（签名文件不存在、任务配置错误）也能一眼看到原因 | ✅ |

Linux 实测（Xvfb，calc）：Make Project 的输出第二行是 `Gradle JDK: JAVA_HOME  Ubuntu version 21.0.12.1  /usr/lib/jvm/java-21-openjdk-amd64`。Windows 部分只单独交叉编译检查过。

## 59. Generate Signed Bundle / APK（2026-10-18）

照 Android Studio 的 Build › Generate Signed Bundle / APK…（`GenerateSignedApkAction`、`GradleSignStep`）：签名信息通过 `android.injected.signing.store.file / store.password / key.alias / key.password` 这几个 Gradle 属性交给构建，覆盖模块自己的 signingConfig。项目没带 `keystore.jks` 也能出签名包。

| 功能 | 状态 |
|---|---|
| Build 菜单项，对话框：App Bundle / APK、应用模块、构建变体（默认 release）、keystore 路径（Choose existing…）、keystore 密码、key 别名、key 密码（密码框可切换显示） | ✅ |
| 校验和 AS 的提示一致（Key store path is not specified 等），改动输入后提示消失 | ✅ |
| 每个模块记住 keystore 路径和别名（设置目录 `signing.json`），密码不保存 | ✅ |
| 构建 `:app:bundleRelease` / `:app:assembleRelease`，成功后弹 “APK(s) generated successfully…” 通知，Locate 打开输出目录 | ✅ |
| Windows 上直接运行 `gradlew.bat`（不再套 `cmd /C`），由 Rust 按 cmd 规则转义参数，密码里的 `&`、`^`、`%`、引号不会被 cmd 解释 | ✅（Windows 未实测） |
| Create new… 打开 New Key Store（AS 的 `NewKeyStoreDialog`）：keystore 路径和密码、key 别名（默认 key0）、密码、有效期（默认 25 年）、证书六项；校验与 AS 一致（密码至少 6 位、两次一致、证书至少填一项）；用 Gradle JDK 的 `keytool -genkeypair` 生成 RSA 2048 密钥，密码走环境变量（`-storepass:env`），不上命令行；成功后回填到签名对话框 | ✅ |
| Destination Folder（AS 的 `GradleSignStep`）：默认是模块目录，每个模块按 APK / Bundle 分别记住上次的目录；通过 `android.injected.apk.location` 交给 Gradle，包落在 `app/release/`（有风味时 `app/demo/release/`），通知的 Locate 打开这里；为空、不存在或不可写时给出 AS 的提示 | ✅ |
| Remember passwords（AS 的 `KeystoreStep`）：勾选后两个密码存进系统的密码库，键名同 AS（`KEY_STORE_PASSWORD__<路径>`、`KEY_PASSWORD__<路径>__<别名>`）；不勾选时删掉已存的；打开对话框或选了 keystore 后在后台读出密码，只填空着的密码框。也记住上次选的 App Bundle / APK | ✅ |
| 密码库：macOS 钥匙串；Linux 用 Secret Service（`secret-tool`），没有时退回设置目录的 `passwords.json`（仅本人可读）；Windows 用 `passwords.json`，内容用 DPAPI（`CryptProtectData`，当前 Windows 用户）加密 | ✅（Linux 实测文件退回；Windows、macOS 需真机验证） |

Linux 实测（Xvfb，假 Android 工程 `droid`）：对话框能列出 app / release；空字段提示正确；填入密码 `p&a^s% w"d` 后构建，Gradle 端 `providers.gradleProperty` 读到的密码和别名与输入完全一致，成功后弹出带 Locate 的通知。

2026-10-20 Linux 实测 Remember passwords 和 Destination Folder：不存在的目录给出 “The destination folder does not exist or is not writeable”；选 `/tmp/…/out` 构建，Gradle 收到 `android.injected.apk.location=/tmp/…/out`，假任务写出 `out/release/app-release.apk`；gnome-keyring 弹出新建钥匙环的框，之后两个密码以 AS 的键名存进 Secret Service；重开对话框时 APK、目标目录、两个密码都回来了；切到 App Bundle 目录变回模块目录；取消勾选再构建后钥匙环里的两条被删掉；`secret-tool` 不可用时写进 `passwords.json`，权限 0600。

## 60. HotSwap / Apply Code Changes（2026-10-20）

照 IntelliJ 的 HotSwap（`HotSwapManager`、`ReloadClassesWorker`、编辑器右上角的浮动工具条 `HotSwapFloatingToolbarProvider`）：调试会话中改了代码，把这次会话以来新编译出的类通过 JDWP `RedefineClasses` 换进正在运行的 VM，不用重启。代码：`src/domain/hotswap.rs`、`src/usecase/hot_swap.rs`、`DebugSession::hot_swap`、`src/infra/class_files.rs`（扫描类文件、d8）、`src/infra/jdwp_client.rs`（`CapabilitiesNew`、`RedefineClasses`）。

| 功能 | 状态 |
|---|---|
| 调试会话中保存 .kt / .java 后，编辑器右上角出现 “Code changed: [Apply HotSwap] ×”；进行中显示 “Reloading classes…”，成功后 “Code has been reloaded ✓”，Hide 只是收起，改动留到下次 | ✅ |
| Apply HotSwap：先用 Gradle 编译改过的源码所在模块（main 用 `classes` / `compileDebugJavaWithJavac`，测试源码用 `testClasses` / `compileDebugUnitTestJavaWithJavac`，变体跟随 Build Variants），再找出上次重载以来写出的类文件（Gradle 和 AGP 的 Java、Kotlin 输出目录），只换 VM 已加载的类 | ✅ |
| 结果用 IntelliJ 的文案：“1 class reloaded”、“Loaded classes are up to date. Nothing to reload.”；失败时 “HotSwap failed” 通知，原因按 JDI 的说法（如 “Operation not supported by VM: add method not implemented”）；编译失败时编译错误进 Problems；Debug 窗口的控制台也记一行 | ✅ |
| 重载后，断在这些类里的行断点重新下（旧代码上的断点随旧代码失效） | ✅ |
| Run 菜单：Apply Changes and Restart Activity（Ctrl+F10）、Apply Code Changes（Ctrl+Alt+F10）、Debugging Actions › Reload Changed Classes | ✅ |
| Android：调试中的应用（Debug 'app' 或 Attach debugger）走同一条路，类文件先用 SDK build-tools 里的 d8（`--file-per-class`，每个类一个 dex）转成 dex 再交给 ART | ✅（未在真机验证） |
| AS 不调试时也能 Apply Changes（靠它自己的部署代理 `installer` / `agent.so` 换代码）；Junction 没有这个代理，所以没挂调试器时 Apply Changes 提示后重新安装并重启应用；Restart Activity 也不做（调试中只换代码） | ⚠️ 差异 |

Linux 实测（Xvfb，Java + JUnit 4 工程）：在 `CalcTest.adds` 第 10 行断住（`first = 3` 已算出），把 `Calc.add` 改成 `a + b + 10` 并保存，浮条出现；点 Apply HotSwap 后显示 “Code has been reloaded”，控制台 “HotSwap: 1 class reloaded”；继续运行后测试输出 `first=3 second=13`。再给 `Calc` 加一个方法后 Apply，得到 “Operation not supported by VM: add method not implemented”，浮条回到 “Code changed:”。

## 61. Project Structure 对话框（2026-10-20）

代码：`src/domain/project_structure.rs`、`src/domain/dependencies.rs`（从构建脚本 / version catalog / `gradle-wrapper.properties` / `local.properties` 读值、按原写法写回）、`src/usecase/project_structure.rs`（读取一个构建、把改动写回各文件）、`src/ui/project_structure_dialog.rs`。

| 功能 | 状态 |
|---|---|
| File › Project Structure…（Ctrl+Alt+Shift+S，macOS ⌘;），左侧 SDK Location / Project / Modules / Dependencies / Build Variants 五页 | ✅ |
| SDK Location：Android SDK location（`local.properties` 的 `sdk.dir`，按 AS 的转义写 `C\:\\Users\\…`；清空则删除该行），“…” 选目录 | ✅ |
| Project：Android Gradle Plugin Version（根脚本的 `id("com.android.*") version`、`buildscript` 的 classpath、settings 脚本的 plugins、catalog 的 `[plugins]` 或其 `version.ref`，改动写到所有出现处）；Gradle Version（改 `distributionUrl` 里的版本） | ✅ |
| Modules：模块列表，右侧 Properties / Default Config / Signing Configs 三个标签（2026-10-21 起按 AS 分成标签）：Properties（Compile Sdk Version、Build Tools Version、NDK Version、Source / Target Compatibility）和 Default Config（Application ID、Min / Target SDK Version、Version Code、Version Name）；Kotlin DSL 和 Groovy 都支持（`x = v`、`x v`、`x(v)`） | ✅ |
| 写回保持原写法：数字、引号（单 / 双）、`JavaVersion.VERSION_1_8` 显示为 `1.8`；`libs.versions.minSdk.get().toInt()`（Groovy 的 `toInteger()`）显示 catalog 里的值，旁边标出 `libs.versions.minSdk`，改动写进 toml；其他表达式（`rootProject.ext.target`）灰显不可改；未写的属性显示 “Not set”，填了值就新增（见下） | ✅ |
| OK：校验（数字字段必须是整数，Java 版本只能是数字和点），出错时在对话框底部显示 “Version Code: 'x' is not a whole number.” 并保持打开；成功后写文件并自动 Sync | ✅ |
| Dependencies（2026-10-20）：左边模块，中间 Declared Dependencies（顶层 `dependencies { }` 里的每条声明：库显示解析后的 `g:a:v`，`libs.x` 经 catalog 解析，模块显示 `:lib`，`platform(…)`、`files(…)` 照写的显示），右边选中项的 Requested Version（写在脚本里就改脚本，`libs.x` 改 catalog 的版本或其 `version.ref` 指向的 `[versions]`）和 Configuration 下拉；“+” 加模块依赖，“−” 删除（连同 `{ exclude … }` 闭包），下方 Add Library Dependency 填 `group:artifact:version` 和配置后添加：项目有 catalog 时按 AS 的做法加到 catalog 里（`libs.okhttp`），否则写坐标；没有 `dependencies` 块时新建；Kotlin DSL 和 Groovy 写法各按原样 | ✅（加库不能在线搜索，要填完整坐标；`buildscript` 里的 classpath 不列出） |
| OK 时先写依赖的改动，再按新文本找回 Properties / Default Config 的位置写属性，两页同时改也不会错位 | ✅ |
| Build Variants（2026-10-20）：左边模块，Build Types / Flavors 两个标签，中间是脚本里声明的构建类型或产品风味（`release { }`、`getByName("debug") { }`、`create("free") { }`），右边按 AS 的行：构建类型有 Debuggable、Jni Debuggable、Minify Enabled、Shrink Resources、Multi Dex Enabled、Application Id Suffix、Version Name Suffix（Kotlin DSL 的 `isX` 和 Groovy 的 `x` 都认），风味有 Dimension、Application ID、Application Id Suffix、Min / Target SDK、Version Code / Name、Version Name Suffix；布尔值只接受 true / false。Default Config 的行现在只在 `defaultConfig { }` 里找，不会读到风味里的同名设置 | ✅ |
| Signing Configs（2026-10-21）：Modules 页的第三个标签，列出 `android { signingConfigs { … } }` 里的签名配置，右边 Store File（`file("keys/release.jks")` 显示和改引号里的路径）、Store Password、Key Alias、Key Password，密码显示成圆点。Build Types 加了 Signing Config 行：Kotlin DSL 的 `signingConfigs.getByName("release")` 和 Groovy 的 `signingConfigs.release` 都显示 `release`，可改 | ✅（Signing Config 是输入框，不是 AS 的下拉） |
| 新增 / 删除（2026-10-21）：Build Types、Flavors、Signing Configs 的名称列上方有名称输入框和 + / −，按 AS 的 + / − 新增或删除构建类型、产品风味、签名配置；新加的项右边的行可直接填。点 OK 时先写块（Kotlin DSL 的 debug / release 用 `getByName("x")`，其余 `create("x")`；Groovy 写 `x { }`；容器块不存在时加在 `android { }` 末尾），再写行；删除去掉整个块。名称必须是标识符且不能重名 | ✅ |
| 新增未写的属性（2026-10-21）：Properties、Default Config、构建类型、风味、签名配置里显示 “Not set” 的行，填值后 OK 会在对应块的末尾加一行，缩进跟块里已有的行，Kotlin DSL 写 `isDebuggable = true`、`minSdk = 24`、`storeFile = file("…")`，Groovy 写 `debuggable true`、`signingConfig signingConfigs.release`；Compile Sdk 等加到 `android { }`，Source / Target Compatibility 加到 `compileOptions { }`（没有这个块就不能加）；空块 `{}` 会展开成多行，单行块用 `;` 接在后面。字符串里的 `"`、`\`、`$` 会转义，读回时还原 | ✅ |
| Suggestions（2026-10-21）：左边 <All Modules> 和各模块（带条数），右边 Updates：每个声明了版本的库有更新的稳定版（预览版可升到更新的预览版，规则同 Gradle 文件里的 newer-version 提示）时一行 `group:artifact:当前版本`，下面写模块和 “Newer version available: x”，点 Update 改成新版本（写在 catalog 里的改 catalog），OK 时写回。版本在第一次打开这页时后台查（Google Maven / Maven Central，结果在磁盘缓存一天） | ✅（只有依赖更新这一类；AS 的其他提示如 AGP 升级不在这页） |

Linux 实测（Xvfb，两个模块的 Groovy 工程 + catalog）：2026-10-21 在 Signing Configs 标签给 release 填 Store Password（含 `"`）和 Key Password、release 构建类型填 Signing Config `release`、debug 填 Debuggable 和 Jni Debuggable、Properties 填 NDK Version 后 OK，`git diff` 正好是这六行新增，位置和缩进正确；重开对话框密码显示为圆点。Build Variants 页把 release 的 Minify Enabled 改 true、free 风味的 Min SDK 改 23 后 OK，只改了这两行，Modules 页的 Default Config 仍显示 24。Dependencies 页改 core-ktx 版本为 1.15.0、删 `:lib`、加 `com.squareup.okhttp3:okhttp:4.12.0` 后 OK，`app/build.gradle` 得到 `implementation libs.okhttp`，toml 加了 `okhttp` 的 `[versions]` 和新建的 `[libraries]` 段。其余三页显示正确（SDK `/opt/android-sdk`、AGP 8.5.0、Gradle 8.7，app 的 Min SDK 显示 24 和 `libs.versions.minSdk`，Target SDK 灰显 `rootProject.ext.target`）；lib 的 Compile Sdk 改 35、Gradle 改 8.9、app 的 Min SDK 改 26 后点 OK，`git diff` 只改了 `lib/build.gradle` 一处、`gradle-wrapper.properties` 的 URL 和 toml 的 `minSdk = "26"`；Version Code 填 `x` 时对话框不关闭并提示错误。

## 62. APK Analyzer（2026-10-21）

| 功能 | 状态 |
|---|---|
| Build › Analyze APK… 选 APK / AAB；Project 窗口双击 `.apk` / `.aab` 也直接打开；在编辑区开一个标签页 | ✅ |
| 顶部 “APK size: 31.7 MB, Download Size: 30.9 MB”；文件树列出 Raw File Size（解压后大小）、Download Size（按下载时的压缩估算：已压缩的条目取压缩后大小，未压缩的条目当场 deflate 估算）、% of Total Download Size；目录汇总子项，每层按大小从大到小 | ✅ |
| 选中 AndroidManifest.xml 或 res 下的二进制 XML：解码成带缩进的 XML，放在只读的 XML 高亮编辑器里；引用按 APK 里的 resources.arsc 写成 `@string/app_name`、`?attr/colorPrimary`（库包的写成 `pkg:type/name`），查不到的仍是 `@ref/0x…` | ✅（`0x01…` 框架资源不解析成 `@android:…`，那要读 SDK 的 android.jar） |
| AAB 里的 XML（`base/manifest/AndroidManifest.xml` 和 res 下的文件是 aapt2 的 protocol buffer `XmlNode`）：同样解码成带缩进的 XML，引用用其中保存的资源名写成 `@string/app_name` | ✅（容器里没有 aapt2 / bundletool，只按 Resources.proto 构造的数据验证过） |
| 选中 `classes*.dex`：“This dex file defines N classes with M methods, and references R methods.”，下面按包展开的类树，列出 Defined Methods / Referenced Methods；只被引用、不在本 dex 定义的包和类灰显 | ✅（没有 AS 的 Size 列，也不能多选几个 dex 合并查看） |
| 文本文件（.properties、.json、.version 等）直接显示内容；其他文件提示没有预览 | ✅ |
| 顶部 Compare with previous APK…：选旧 APK，标签页换成 APK Size Differ：“old.apk → new.apk: 30 MB → 32 MB (+2.0 MB)”，文件树合并两边，列出 Old Size / New Size / Diff Size（Raw 大小），每层按增长从多到少；Close Comparison 回到分析页 | ✅（没有 AS 的 Show only changed / file-by-file patch 大小） |

Linux 实测：termux-app 0.118.1 的 debug APK（33 MB，755 个条目，27 个 dex）后台算大小约 0.1 秒，`classes.dex`（9.3 MB）的汇总约 0.07 秒，Manifest 解码正确。resources.arsc 解析（2026-10-21 补）：同一 APK 的 3521 个资源名约 7 毫秒，Manifest 里的 `@string/application_name`、`@mipmap/ic_launcher` 等都换成了名字；和改过的旧版本对比，新增、删除、变大的文件都对上。

## 63. SDK Manager（2026-10-21）

| 功能 | 状态 |
|---|---|
| Tools › SDK Manager 打开 Settings › Languages & Frameworks › Android SDK；Android SDK Location 默认取项目 local.properties 的 sdk.dir、ANDROID_HOME、ANDROID_SDK_ROOT、默认安装目录；改路径回车后重新扫描 | ✅（路径不写回 local.properties） |
| 已安装的包：扫 SDK 目录下各包的 `package.xml`（最深 4 层，system-images 也能认） | ✅ |
| 可装的包：Google 的 `repository2-3.xml`，只取 stable 频道、每个 path 的最新版；下载失败时用上次缓存的列表并提示 | ✅（system-images、add-ons 等其他仓库的列表没拉） |
| SDK Platforms 标签：每个 API 一行（“Android 14.0 ("UpsideDownCake")”、API Level、Revision、Status）；Show Package Details 展开成 Platform、Sources 等单个包 | ✅ |
| SDK Tools 标签：Build-Tools、NDK、CMake 等多版本的工具合成一行显示最新版；Show Package Details 列出每个版本；Hide Obsolete Packages（已装的过时包仍显示） | ✅ |
| 勾选：未装的勾上=安装；已装的取消=删除；有更新的已装包再点一次=更新（Status 显示 Will be installed / updated / removed） | ✅（AS 是三态复选框，这里用 Status 文字区分） |
| Apply / OK：Confirm Change 列出要装和要删的包及下载大小 → License Agreement（未接受的许可证，接受后写 `licenses/<id>` 的 SHA-1，Gradle 也认）→ SDK Component Installer（逐个下载、SHA-1 校验、去掉 zip 根目录解压、写 package.xml，进度条和日志，可取消） | ✅ |
| `JUNCTION_SDK_REPO` 环境变量换成镜像地址 | ✅ |

容器连不上 dl.google.com，只用本地 HTTP 假仓库（12 个包）验证过：装 Platform 35 + Sources、更新 Platform 34、装 Build-Tools 35 / Platform-Tools、删 Build-Tools 34，许可证文件和 package.xml 都正确。

## 64. Layout Inspector（2026-10-21）

| 功能 | 状态 |
|---|---|
| View › Tool Windows / Tools › Layout Inspector：右侧工具窗口，设备下拉、窗口下拉（多个 Activity 时）、Refresh Layout、Live Updates（约 2.5 秒刷新一次）、Hide；打开时自动给第一台设备拍快照 | ✅ |
| 快照：默认读 `dumpsys activity top` 的 View Hierarchy（不占用 UI Automator，Maestro 在跑也能用），坐标按父 View 累加成屏幕坐标；勾 UI Automator 改用 `uiautomator dump`（能看到 Compose 节点和文字），被别的工具占用时提示原因 | ✅ |
| Component Tree：类名 + `@id/…` + 文字，可折叠；GONE / INVISIBLE 的 View 灰显 | ✅ |
| 中间是设备截图，每个可见 View 画框；悬停高亮、点击选中最里层的 View，树里同步展开并滚到它；刷新后保留选中 | ✅ |
| Attributes：layout / bounds、size、visibility、focusable、enabled、clickable、focused、selected、pressed 等，UI Automator 模式另有 text、content-desc、checked、scrollable 等 | ✅（AS 的完整属性和 Declared Attributes 需要设备端 agent，没做） |
| 3D（标题栏 3D 按钮）：每个可见 View 按层级深度拉开成一层，默认斜着看（AS 进入 3D 时的角度）；拖动左右 / 上下旋转，Layer Spacing − / + 调层间距，Reset View 复位；悬停和点击选中最上面那层的 View，与树同步 | ✅（层上没有贴截图，只画半透明的层和边框） |
| Compose 重组计数、改属性 | ❌ |

只用模拟 adb 验证过（假 `dumpsys activity top` 输出 + 540×1200 截图）：树、框选、属性、窗口切换和 UI Automator 失败提示都正确。

## 65. Profiler（2026-10-21）

| 功能 | 状态 |
|---|---|
| View › Tool Windows / Tools › Profiler：底部工具窗口，设备下拉、进程下拉（`ps -A` 里的应用进程，本项目的 applicationId 排在最前）、Start / Stop | ✅ |
| CPU 时间线：每秒读 `/proc/stat` 和 `/proc/<pid>/stat`，算应用占整机 CPU 的百分比，显示当前值和线程数；保留最近 2 分钟 | ✅ |
| Memory 时间线：每秒读 `dumpsys meminfo <pid>` 的 App Summary，按 Java / Native / Graphics / Stack / Code / Others 堆叠，图例显示 Total 和各项当前值（AS 的配色和顺序） | ✅ |
| 进程退出后自动停止并提示 | ✅ |
| Capture heap dump：`am dumpheap` 后拉回电脑，存到 `~/.cache/Junction/captures/<进程>_<时间>.hprof`（AS 的 File › Open 也能打开）；解析成类列表：Class Name、Allocations、Shallow Size、Retained Size（从 GC Root 建支配树，每个对象的保留大小按类相加；`Reference.referent` 不算持有，和 AS 一样），默认按 Retained Size 从大到小排，按堆（app / image / zygote）切换，可搜索 | ✅（JVM 导出的 78 MB hprof、约 120 万对象解析约 1.2 秒，峰值内存约 290 MB） |
| 点一个类：右侧 Instance List（Instance `HashMap@0x…`、Depth、Shallow Size、Retained Size，可达的按 Retained 从大到小，不可达的灰显）；点一个实例：下方 References 树，每行“字段 in 引用者”（`next`、`[3]`、`static sInstance`），按 Depth 从小到大，可一层层展开到 GC root（标 “GC root”，环不再展开，每层最多 200 个） | ✅（没有 Fields 标签页、Jump to Source 和按包分组） |
| Record（CPU 方法追踪）：录制方式下拉 Java/Kotlin Method Sample（`am profile start --sampling 1000`）/ Java/Kotlin Method Trace（不带 `--sampling`，记录每次调用），Stop recording 后 `am profile stop` 并拉回 `.trace`，存到 `~/.cache/Junction/captures/<进程>_<时间>.trace` | ✅ |
| 解析 ART 追踪文件（文本头 + `SLOW` 二进制记录，版本 1–3，单时钟和双时钟），每个线程建调用树；录制开始前已在运行的方法从第一条记录算起，结束时还没返回的算到最后一条 | ✅ |
| Call Chart（默认标签页）：按时间画出每次调用，调用者在上、被调用者在下，颜色同 Flame Chart；Ctrl + 滚轮缩放、Shift + 滚轮（或横向滚轮）平移、Zoom In / Zoom Out / Reset Zoom，点一个调用放大到它，悬停显示方法、耗时和起止时间 | ✅ |
| 追踪视图：线程下拉（按耗时排序）、Wall Clock Time / Thread Time 切换；Top Down（Name、Total、Self、Children 及各自占线程的 %，默认沿最耗时的路径展开）、Bottom Up（每个方法的总耗时，展开看调用者；递归不重复计算）、Flame Chart（相同调用栈合并，宽度按耗时，绿色是本项目代码，橙色是系统代码，蓝色是其他库，悬停显示耗时） | ✅（没有框选时间段、Summary、Jump to Source 和搜索） |
| 系统跟踪（Perfetto）：见第 69 节 | ✅ |
| C/C++ 采样（simpleperf）、分配跟踪、Energy | ❌ |

只用模拟 adb 验证过（假 `/proc`、`dumpsys meminfo` 输出和一个 120 万字节的 Android 格式 hprof）：曲线、图例、进程切换、堆转储列表、堆切换和搜索都正确。方法追踪只用脚本按 ART 格式生成的追踪文件（4 个线程、约 1.8 万条记录）验证过，没有真机录制过。

## 66. App Inspection：Database Inspector（2026-10-21）

| 功能 | 状态 |
|---|---|
| View › Tool Windows / Tools › App Inspection：底部工具窗口，设备下拉、应用下拉（正在运行的应用，`:remote` 这类子进程合并到包名，本项目的 applicationId 排在最前，运行中时自动选中） | ✅ |
| 选中应用后用 `run-as <包名>` 列出并读取 `databases/` 下的所有数据库（带上 `-wal` 文件里已提交的页，Room 默认 WAL 模式也能看到最新数据）；左侧数据库 → 表的树 | ✅ |
| 点表显示行：列名来自 CREATE TABLE，`INTEGER PRIMARY KEY` 列填 rowid，NULL 斜体，BLOB 显示 `X'..'`，长文本（溢出页）和 UTF-16 编码库都能读；最多读 5000 行，网格可横向滚动 | ✅ |
| Refresh：重新从设备读取，保留当前表 | ✅ |
| 应用不可调试（`run-as` 拒绝）时显示原因 | ✅ |
| 查询栏：选数据库，输入 SELECT 回车或点 Run，结果显示在网格里（Refresh / Live updates 时重新执行）。支持多表 `JOIN … ON` / `USING`、`LEFT JOIN`、`CROSS JOIN` 和逗号连接（等值条件走哈希连接），表别名和 `表.列`、`表.*`；表达式（四则运算、`%`、`||`、比较、`LIKE`、`IN`、`BETWEEN`、`IS [NOT] NULL`、`CASE`、`CAST`，列与字面量比较时按列的类型转换）；函数 `lower` / `upper` / `length` / `substr` / `trim` / `replace` / `instr` / `round` / `abs` / `coalesce` / `ifnull` / `nullif` / `iif` / `typeof` / `hex` 等；聚合 `COUNT` / `SUM` / `AVG` / `MIN` / `MAX` / `TOTAL` / `GROUP_CONCAT`（可带 `DISTINCT`）、`GROUP BY`、`HAVING`、`SELECT DISTINCT`、`ORDER BY`（列、表达式、别名、序号）、`LIMIT` / `OFFSET`、`AS` 别名；不支持的写法（子查询、`UNION`、`WITH`、窗口函数、未知函数）报出原因；INSERT / UPDATE / DELETE 等会说明数据库是副本、改不了应用的数据 | ✅（自己写的求值，不是 SQLite；结果和 Python sqlite3 对比过） |
| Live updates：每 3 秒从设备重新读取数据库，保留当前表或查询 | ✅（AS 是表变化时才推送；这里是定时整库重读，大库会慢） |
| `WITHOUT ROWID` 表：按主键 B 树读出（主键列在前，再按声明顺序还原列），行按主键排序，查询也能用 | ✅（2026-10-21） |
| 导出：网格上方 Export to CSV…（当前表或查询的全部行，不受 5000 行限制；NULL 为空、BLOB 为十六进制、按 RFC 4180 加引号）、Export Database…（整个数据库文件，WAL 里已提交的页写进去，单独就能打开） | ✅（2026-10-21；AS 还能导出成 SQL，这里没有） |
| 编辑单元格 | ❌（会写应用的数据库，暂不做） |
| Network Inspector | 🟡 见第 68 节 |
| Background Task Inspector | ✅ 见第 67 节 |

数据库是自己写的只读 SQLite 解析（`src/domain/sqlite_file.rs`），不依赖 sqlite 库，也不需要在设备上装 agent。只用模拟 adb 验证过（`sqlite3` 生成的普通库和带 WAL 的库）。

## 67. App Inspection：Background Task Inspector（2026-10-21）

| 功能 | 状态 |
|---|---|
| App Inspection 顶部切换 Database Inspector / Background Task Inspector，两者共用设备和应用下拉、Refresh、Live updates（每 3 秒重读） | ✅ |
| 一张表按 Workers / Jobs / Alarms / WakeLocks 分组（可折叠），列为 Class Name、Status、Start、Result；Worker 状态按 AS 着色（Running 蓝、Succeeded 绿、Failed 红） | ✅ |
| Workers：`run-as` 读取应用的 `androidx.work.workdb`（WorkSpec、WorkTag、WorkName、Dependency 表），解码输入输出 `Data`（WorkManager 2.9 起的格式和旧版 Java 序列化格式都支持） | ✅ |
| Worker 详情：类名、标签、唯一名、UUID、约束、One Time / Periodic 频率、状态、初始延迟、加急、前后依赖（可点击跳转）、开始时间、重试次数、输入和输出数据 | ✅ |
| 按标签过滤 Workers | ✅ |
| Jobs：`dumpsys jobscheduler <包名>`，显示 Running / Ready / Waiting、所需和未满足的约束、周期、入队时间、运行时间窗口，标出 WorkManager 调度的 job | ✅ |
| Alarms：`dumpsys alarm` 里本应用的闹钟（类型、tag、多久后触发、窗口、重复间隔、PendingIntent） | ✅ |
| WakeLocks：`dumpsys power` 里本应用 uid（`pm list packages -U`）持有的 wake lock | ✅ |
| 取消 Worker、已结束的 job / alarm 历史、Worker 链的图形视图、跳到入队代码 | ❌（AS 靠应用内 agent 实时上报；这里只读设备当前状态） |

解析在 `src/domain/background_tasks.rs`，只用模拟 adb 和 `sqlite3` + Java 生成的 WorkManager 库验证过，没在真机上看过 dumpsys 的实际输出。

## 68. App Inspection：Network Inspector（2026-10-21）

| 功能 | 状态 |
|---|---|
| App Inspection 顶部第三个标签 Network Inspector：选中应用后每秒从设备的 `/proc/net/tcp`、`tcp6`、`udp`、`udp6` 读取该应用 uid（`pm list packages -U`）的套接字；只在工具窗口显示且在这个标签时读取 | ✅ |
| 连接表：协议、远端地址（IPv4 映射的 IPv6 显示为 IPv4）、本地地址、状态（ESTABLISHED、TIME_WAIT 等）、打开时间、持续时长；按套接字 inode 跨次读取追踪，关掉的连接留在表里标 Closed，Clear 清掉已关闭的 | ✅ |
| 默认只列有远端的连接，勾选后也显示监听和未连接的套接字 | ✅ |
| AS 的 HTTP 请求列表（URL、状态码、请求 / 响应头和正文、时间线）、流量曲线、主机名 | ❌（AS 靠应用内 agent 拦截 OkHttp / HttpURLConnection；不装 agent 拿不到） |

解析在 `src/domain/network_connections.rs`，只用模拟 adb 验证过。部分设备可能不让 adb shell 读 `/proc/net`，这时显示设备返回的错误。

## 69. Profiler：System Trace（2026-10-21）

| 功能 | 状态 |
|---|---|
| Record 的录制方式下拉多了 System Trace：在设备上用 `perfetto --background` 录制（配置从标准输入传入：`sched_switch`、`sched_waking`、`task_newtask`、`task_rename`、`cpu_frequency`、`ftrace/print`，atrace 的 am / dalvik / gfx / input / view / wm 类别和本应用的 `Trace.beginSection`，开始时扫描所有进程和线程名；64 MB 环形缓冲，最长 10 分钟）；Stop recording 时 `kill -TERM` 让 perfetto 写出并拉回，存到 `~/.cache/Junction/captures/<进程>_<时间>.pftrace`（Perfetto UI 也能打开） | ✅ |
| 解析 Perfetto 的 protobuf trace（自己写的读取，不依赖 protobuf 库）：按时间合并各 CPU 的 ftrace 事件，算出每个 CPU 上跑过的线程、频率变化，本应用每个线程的状态（Running、Runnable、Sleeping、Waiting（不可中断，通常是 I/O））和 atrace 区段（嵌套深度）；录制中新建和改名的线程也能归到本应用 | ✅ |
| CPU cores：每个核一条泳道，本应用的线程绿色、其他进程灰色，下面一条橙色频率条；悬停显示时间点、线程名（其他进程带进程名）、这段运行时长和频率 | ✅ |
| Threads：本应用的线程（主线程在前，其余按运行时间排序），每个线程一条状态条（绿 Running、蓝 Runnable、橙 Waiting，Sleeping 只画细线）加区段；悬停显示状态和时长，点区段放大到它；点线程名后底部汇总它在可见范围里各状态的时间 | ✅ |
| Ctrl + 滚轮缩放、Shift + 滚轮平移、Zoom In / Zoom Out / Reset Zoom | ✅ |
| Frames（Janky frames）、SurfaceFlinger / 显示泳道、框选时间段汇总、Flame Chart / Top Down、Energy、`compact_sched` 格式 | ❌（配置里关掉了 compact_sched，所以录制的 trace 都能读） |

解析在 `src/domain/system_trace.rs`，只用构造的 trace 验证过（单元测试，以及脚本生成的 8 核、3 秒、约 80 万字节的 trace 走模拟 adb 的完整录制流程）。没有在真机上录制过，perfetto 需要 Android 9 及以上。
