# 参与贡献 · Contributing

## 反馈问题

请说明你想完成什么、在哪一步遇到问题，以及实际看到的结果。连接问题请注明两端系统版本和网络连接方式。附截图或日志前，请遮住配对码、设备地址、文件名和私人输入。[提交 Issue](https://github.com/kuiwaiwai/plumelink/issues)

## 提交改动

较大的功能或协议变更建议先开 Issue 讨论。请让每个 PR 聚焦一个问题，说明改动原因和测试方法。修改界面文案时，请检查简体中文、繁體中文和英文三种语言；修改用户可见行为时，请同步相关使用说明。

提交前运行 `npm run verify:public-source` 检查将进入仓库的文件，再运行与改动相关的测试。

<a id="build-zh"></a>

## 从源码运行

在 Windows x64 上准备 Node.js 22.12+、Rust 1.98+（MSVC）、Visual C++ Build Tools、Windows SDK 和 WebView2。构建手机端还需要 JDK 17、Android SDK Platform 35 和 Build Tools 35.0.0。

在仓库根目录运行：

```powershell
npm ci
rustup target add wasm32-unknown-unknown
npm run dev
```

桌面端启动后，在另一个终端运行 `npm run build:android`，将 `android-app/app/build/outputs/apk/debug/app-debug.apk` 安装到手机。

配置 Android 发布签名后运行 `npm run pack`。`release/` 包含签名 APK 和附带 VB-CABLE 驱动的 Windows 安装程序。

## Reporting an issue

Describe what you were trying to do, the steps to reproduce the problem and what happened. For connection issues, include both operating system versions and how the devices connect to the network. Redact pairing codes, device addresses, filenames and private input from screenshots and logs. [Open an issue](https://github.com/kuiwaiwai/plumelink/issues)

## Pull requests

Please discuss substantial features or protocol changes in an issue first. Keep each PR focused and explain why the change is needed and how you tested it. Check UI text in all three supported languages and update the relevant guide when behavior changes.

Run `npm run verify:public-source` before committing, along with the tests that cover your change.

<a id="build-en"></a>

## Build from source

On Windows x64, install Node.js 22.12+, Rust 1.98+ (MSVC), Visual C++ Build Tools, the Windows SDK and WebView2. The Android build also requires JDK 17, Android SDK Platform 35 and Build Tools 35.0.0.

Run these commands from the project root:

```powershell
npm ci
rustup target add wasm32-unknown-unknown
npm run dev
```

With the desktop app running, open a second terminal in the project root and run `npm run build:android`. Install `android-app/app/build/outputs/apk/debug/app-debug.apk` on your phone.

With Android release signing configured, run `npm run pack`. The `release/` directory contains the signed APK and the Windows installer with the VB-CABLE driver.
