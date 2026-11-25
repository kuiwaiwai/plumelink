# 第三方组件

羽连采用 MIT 许可证，源码版权声明见 [LICENSE](LICENSE)。下列组件各自适用原作者的许可。

| 组件 | 许可 | 用途 |
| --- | --- | --- |
| Tauri 2 / tauri-plugin-single-instance | MIT 或 Apache-2.0 | 桌面窗口、托盘、受限 IPC、单实例 |
| windows / windows-core | MIT 或 Apache-2.0 | 微软维护的 Rust Windows API 绑定 |
| axum / tokio | MIT | Rust HTTP/WebSocket 和异步运行时 |
| serde / serde_json / getrandom / sha2 | MIT 或 Apache-2.0（具体以原许可为准） | 协议与摘要签名 |
| Vue / qrcode / @tauri-apps/api / @lucide/vue | MIT / ISC | 两端页面、图标、二维码、桌面桥接 |
| OkHttp 4.12.0 / Okio 3.6.0 | Apache-2.0 | Android 有界音频 WebSocket、证书绑定 |
| ZXing Android Embedded 4.3.0 / ZXing Core 3.4.1 | Apache-2.0 | Android 离线二维码扫描 |
| AndroidX Core 1.6.0 / Fragment 1.3.6 及传递依赖 | Apache-2.0 | 扫码相机权限及 Android 兼容支持，具体版本见 Gradle 锁文件 |
| Kotlin 标准库 1.9.10 / JetBrains annotations 13.0 | Apache-2.0 | Android 网络库的传递依赖 |
| Zod | MIT | 共享 TypeScript 协议与契约验证 |
| Vite / Playwright / TypeScript | 各自 MIT 或 Apache-2.0 | 仅开发构建和验证工具 |
| Microsoft Edge WebView2 Runtime | Microsoft WebView2 分发条款 | 系统 WebView；缺少运行时才由安装向导联网下载 |
| Android Debug Bridge / AdbWinApi / AdbWinUsbApi | Apache-2.0 及随附 NOTICE 中的许可 | Windows 内置 USB 连接组件 |

## 随包许可

Windows 安装版附带 `THIRD_PARTY_LICENSES.txt`。Windows 主程序内嵌项目与依赖许可，运行时加上 `--licenses` 可将许可文本输出到终端或文件。Android 依赖的许可文本位于 APK 的 `assets/THIRD_PARTY_LICENSES-ANDROID.txt`。

USB 组件取自 Google 官方 Android Platform-Tools，保留原始二进制及完整的 `usb/NOTICE.txt`，不附带 Android SDK 安装器。[ADB 源码与许可](https://android.googlesource.com/platform/packages/modules/adb/) · [Platform-Tools](https://developer.android.com/tools/releases/platform-tools)

## 可选音频驱动

VB-CABLE 由 VB-Audio 提供，采用捐赠授权。Windows 安装包内附 VB-CABLE 官方原始压缩包。用户可在「偏好设置 → 手机麦克风」中启动驱动安装向导。驱动的使用与分发适用 VB-Audio 的许可条款。[VB-CABLE 官网](https://vb-cable.com/) · [VB-Audio 分发与授权条款](https://vb-audio.com/Services/licensing.htm)

欢迎通过捐赠或购买许可支持 VB-Audio。若在单位等专业环境中部署，且最终用户无法单独识别或支持 VB-CABLE，分发者应按部署数量购买相应许可。[VB-Audio 授权说明](https://vb-audio.com/Services/licensing.htm)
