# MonitorDDC

MonitorDDC 是一款 Windows 显示器调节工具，通过 DDC/CI 控制外接显示器的亮度和对比度，并可识别品牌、型号、分辨率、刷新率和连接接口。

## 使用

从 GitHub Releases 下载 `MonitorDDC.exe` 后直接双击运行，无需安装其他运行库。

程序运行时会驻留在 Windows 系统托盘：

- 单击托盘图标可显示或隐藏主窗口
- 点击窗口关闭按钮只会隐藏到托盘，不会退出程序
- 右键托盘图标可选择“打开窗口”“重新扫描显示器”或“退出”
- 勾选“开机自动启动”后，登录 Windows 时会静默启动并直接驻留托盘

使用前请确认：

- 系统为 64 位 Windows 10 或 Windows 11
- 显示器菜单中的 DDC/CI 已开启
- 显卡驱动已正确安装

扩展坞、KVM、转接器或部分笔记本内置屏幕可能不支持 DDC/CI。连接异常时可点击“重新扫描”，或尝试直连显示器。

## 命令行

这些命令可以在 Windows 命令提示符（CMD）或 PowerShell 中使用。

CMD：

```cmd
cd /d D:\tianwen_project\GitHub_project\MonitorDDC\target\release
MonitorDDC.exe --list
MonitorDDC.exe --get-brightness
MonitorDDC.exe --monitor 0 --brightness 30
MonitorDDC.exe --help
```

PowerShell：

```powershell
# 显示器列表及当前参数
.\MonitorDDC.exe --list

# 读取亮度和对比度
.\MonitorDDC.exe --get-brightness --get-contrast

# 调节所有显示器
.\MonitorDDC.exe --brightness 30 --contrast 50

# 调节指定显示器
.\MonitorDDC.exe --monitor 0 --brightness 30
```

CMD 中直接输入 `MonitorDDC.exe`，PowerShell 中使用 `.\MonitorDDC.exe`。亮度和对比度取值范围为 `0` 到 `100`。不带参数运行或双击 exe 时打开图形界面。

`MonitorDDC.exe --tray` 会隐藏主窗口并直接驻留托盘，主要供开机自动启动使用。

## 编译

需要安装：

- [Rust stable MSVC 工具链](https://rustup.rs/)，Rust 1.85 或更高版本
- [Visual Studio 2022 Build Tools](https://visualstudio.microsoft.com/downloads/)，勾选“使用 C++ 的桌面开发”和 Windows SDK

首次编译需要联网下载 Rust 依赖。在项目目录运行：

```powershell
rustup default stable-x86_64-pc-windows-msvc
cargo build --release --locked
```

输出文件：

```text
target\release\MonitorDDC.exe
```

程序使用静态 MSVC CRT，发布时只需要这个 exe。程序 Logo 已保存在 `assets/` 并由 `build.rs` 自动嵌入。

Python 不是程序运行或正常编译所需的依赖。`tools\make_icon.py` 只是以后更换 Logo 时使用的可选工具。

## GitHub

`target/` 和 `.xwin-cache/` 属于本地缓存，已写入 `.gitignore`。建议将源码提交到仓库，把 `MonitorDDC.exe` 上传到 GitHub Releases。
