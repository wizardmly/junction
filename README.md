# Junction Studio

Junction Studio — Git & Code Navigator：桌面客户端，一比一复刻 Android Studio / IntelliJ 的 Git 功能，以及跨语言的代码索引和跳转（JNI、Dart FFI、Rust extern C、Swift/ObjC 桥接）。
技术栈：Rust + [GPUI](https://www.gpui.rs) + [GPUI Kit / gpui-component](https://github.com/longbridge/gpui-kit)。
Git 操作和 IntelliJ 的 git4idea 一样，直接调用 `git` 命令行。

![Log](docs/screenshots/log-dark.png)

## 下载

每次推送到 master，GitHub Actions 会自动编译三个平台的安装包，发布在 [Releases › latest](https://github.com/wizardmly/junction/releases/tag/latest)（链接固定不变）：

| 平台 | 文件 | 用法 |
|---|---|---|
| Windows x64 | `junction-windows-x64.zip` | 解压后运行 `Junction Studio/junction.exe` |
| macOS（Apple 芯片和 Intel 通用） | `junction-macos-universal.dmg` / `junction-macos-universal.app.zip` | 把 Junction Studio 拖进"应用程序" |
| Linux x64 | `Junction-x86_64.AppImage` / `junction-linux-x64.tar.gz` | `chmod +x` 后运行 AppImage，或解压 tar.gz |

各文件的校验值在 `SHA256SUMS.txt` 中。

macOS 版没有经过 Apple 公证，首次打开会提示"Apple 无法验证 Junction Studio.app 是否包含恶意软件"。可以在 系统设置 › 隐私与安全性 里点"仍要打开"，或者执行：

```sh
xattr -dr com.apple.quarantine "/Applications/Junction Studio.app"
```

也可以按下面的步骤在自己的 Mac 上编译，本机编译的应用不会出现这个提示。

## 在 macOS 上编译 Junction Studio.app

1. 从 App Store 安装 **Xcode**，装好后打开一次并同意许可协议。编译需要 Xcode 自带的 Metal 着色器编译器，只装 Command Line Tools 不够。
2. 安装 Rust：

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. 下载代码，编译并安装到"应用程序"：

   ```sh
   git clone https://github.com/wizardmly/junction.git
   cd junction
   packaging/build-macos-app.sh --install
   ```

   不加 `--install` 时只生成 `target/Junction Studio.app`。第一次编译需要 10 到 15 分钟，之后会快很多。

以后更新：在 `junction` 目录执行 `git pull`，再运行一次 `packaging/build-macos-app.sh --install`。

## 运行

需要 Rust 1.90+ 和 `git`。

```sh
cargo run --release -- /path/to/repo     # 不传路径时打开当前目录
JUNCTION_THEME=light cargo run           # 亮色主题
```

- **Windows**：需要 Visual Studio Build Tools（MSVC）。Windows 11 上窗口使用 Mica 背景。
- **macOS**：需要 Xcode（见上文）。
- **Linux**：先安装依赖：`sudo apt install libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libvulkan1 libfontconfig-dev libssl-dev`。

## 功能清单

完整对照表和进度见 [docs/FEATURES.md](docs/FEATURES.md)（M1–M6 阶段）。

## 结构

```
src/git/        git 后端：命令执行与 Console 记录、log 解析、提交图布局、refs、status/commit
src/model.rs    仓库状态（后台加载、选中提交、操作 + 通知）
src/ui/         界面：workspace（主窗口）、log_view、commit_view、branches_popup、diff_view、dialogs
src/theme.rs    IntelliJ Int UI 亮/暗配色，半透明面板
```

`cargo test` 运行解析器、提交图布局和文件树的单元测试。
