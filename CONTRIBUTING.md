# 参与贡献 · Contributing

## 反馈问题

请说明你想完成什么、在哪一步遇到问题，以及实际看到的结果。连接问题请注明两端系统版本和网络连接方式。附截图或日志前，请遮住配对码、设备地址、文件名和私人输入。[提交 Issue](https://github.com/kuiwaiwai/plumelink/issues)

## 提交改动

较大的功能或协议变更建议先开 Issue 讨论。请让每个 PR 聚焦一个问题，说明改动原因和测试方法。修改界面文案时，请检查简体中文、繁體中文和英文三种语言；修改用户可见行为时，请同步相关使用说明。

提交前运行 `npm run verify:public-source` 检查将进入仓库的文件，再运行与改动相关的测试。

<a id="build-zh"></a>

## 从源码运行

在 Windows x64 上准备 Node.js 22.12+、Rust 1.98+（MSVC）、Visual C++ Build Tools、Windows SDK 和 WebView2。构建手机端还需要 JDK 17、Android SDK Platform 35 和 Build Tools 35.0.0。

Windows 构建会自动下载并校验固定版本的 USB 组件，后续构建复用本地缓存。版本与 SHA-256 见 `scripts/usb-component.json`。直接使用 Cargo 前，先运行 `node scripts/prepare-usb.mjs`。

在仓库根目录运行：

```powershell
npm ci
rustup target add wasm32-unknown-unknown
npm run dev
```

桌面端启动后，在另一个终端运行 `npm run build:android`，将 `android-app/app/build/outputs/apk/debug/app-debug.apk` 安装到手机。

Android 原生界面位于 `android-app/`，`apps/mobile/` 保留其界面状态、业务和通信逻辑，通过隐藏 WebView 与原生层交互，不提供独立网页界面。`npm run verify:mobile` 检查原生桥接与通信；界面布局和系统交互需要在 Android 设备上验证。

`npm run verify:launcher` 检查快捷启动失败、状态查询延迟和三语言提示，使用隔离入口，不启动实际应用。

`npm run verify:window-picker` 检查窗口列表自动更新、触摸期间暂缓更新，以及离开页面后停止刷新，覆盖三种语言。

`npm run verify:microphone` 检查麦克风状态指导、通道归属、保留错误提示及关闭后停止查询，覆盖三种语言；音频设备效果使用模拟实现。

`npm run verify:shortcut-plans` 检查三语言的快捷方案保存、预览、切换、重命名、删除和恢复；方案管理不执行远控按键。

前端构建后，运行 `npm run verify:desktop-ui -- --connection-states` 检查电脑端有线、无线和停止状态下的引导、配对信息和诊断报告。检查使用模拟 Tauri 接口，结果保存在独立目录中，不执行 Windows 输入。

配置 Android 发布签名后运行 `npm run pack`。`release/` 包含签名 APK 和附带 VB-CABLE 驱动的 Windows 安装程序。

验收候选版本时运行 `npm run pack -- -Candidate`，两个安装包会保存到 `artifacts/candidates/` 下的独立目录，不替换 `release/`。

## Reporting an issue

Describe what you were trying to do, the steps to reproduce the problem and what happened. For connection issues, include both operating system versions and how the devices connect to the network. Redact pairing codes, device addresses, filenames and private input from screenshots and logs. [Open an issue](https://github.com/kuiwaiwai/plumelink/issues)

## Pull requests

Please discuss substantial features or protocol changes in an issue first. Keep each PR focused and explain why the change is needed and how you tested it. Check UI text in all three supported languages and update the relevant guide when behavior changes.

Run `npm run verify:public-source` before committing, along with the tests that cover your change.

<a id="build-en"></a>

## Build from source

On Windows x64, install Node.js 22.12+, Rust 1.98+ (MSVC), Visual C++ Build Tools, the Windows SDK and WebView2. The Android build also requires JDK 17, Android SDK Platform 35 and Build Tools 35.0.0.

Windows builds download and verify a pinned USB component, then reuse the local cache. The version and SHA-256 values are in `scripts/usb-component.json`. Before invoking Cargo directly, run `node scripts/prepare-usb.mjs`.

Run these commands from the project root:

```powershell
npm ci
rustup target add wasm32-unknown-unknown
npm run dev
```

With the desktop app running, open a second terminal in the project root and run `npm run build:android`. Install `android-app/app/build/outputs/apk/debug/app-debug.apk` on your phone.

Native Android views live in `android-app/`. The `apps/mobile/` directory contains their state, business logic and communication runtime, connected through a hidden WebView. It does not provide a standalone web interface. Run `npm run verify:mobile` to check the native bridge and communication; validate layouts and system interactions on an Android device.

Run `npm run verify:launcher` to check launch failures, delayed status checks and messages in all three languages. It uses isolated entries without launching applications.

Run `npm run verify:window-picker` to check live window lists, updates deferred during touch, and polling suspension off the page in all three languages.

Run `npm run verify:microphone` to check status guidance, stream ownership, preserved error messages and polling suspension in all three languages. Audio device effects are simulated.

Run `npm run verify:shortcut-plans` to check saving, previewing, applying, renaming, deleting and restoring shortcut plans in all three languages without executing remote keys during plan management.

After building the frontend, run `npm run verify:desktop-ui -- --connection-states` to check desktop guidance, pairing details and diagnostic reports for USB, wireless and stopped states. It uses a simulated Tauri bridge, saves results in a unique directory and does not send Windows input.

With Android release signing configured, run `npm run pack`. The `release/` directory contains the signed APK and the Windows installer with the VB-CABLE driver.

For a candidate build, run `npm run pack -- -Candidate`. Both packages go into a unique directory under `artifacts/candidates/`, leaving `release/` unchanged.
