# Third-party notices

PlumeLink is distributed under the MIT license; see [LICENSE](LICENSE). Each component below retains its own license and attribution requirements.

| Component | License | Purpose |
| --- | --- | --- |
| Tauri 2 / tauri-plugin-single-instance | MIT or Apache-2.0 | Desktop windows, tray, restricted IPC and single-instance support |
| windows / windows-core | MIT or Apache-2.0 | Microsoft-maintained Rust bindings to Windows APIs |
| axum / tokio | MIT | HTTP, WebSocket and asynchronous runtime |
| serde / serde_json / getrandom / sha2 | MIT or Apache-2.0; see each component’s license | Serialization, randomness and hashing |
| Vue / qrcode / @tauri-apps/api / @lucide/vue | MIT / ISC | Interface runtime, icons, QR codes and desktop bridge |
| OkHttp 4.12.0 / Okio 3.6.0 | Apache-2.0 | Android audio WebSocket and certificate pinning |
| ZXing Android Embedded 4.3.0 / ZXing Core 3.4.1 | Apache-2.0 | Offline QR scanning on Android |
| AndroidX Core 1.6.0 / Fragment 1.3.6 and transitive dependencies | Apache-2.0 | Camera permissions and Android compatibility; versions are recorded in Gradle lockfiles |
| Kotlin standard library 1.9.10 / JetBrains annotations 13.0 | Apache-2.0 | Transitive dependencies of Android networking libraries |
| Zod | MIT | Shared TypeScript protocol validation |
| Vite / Playwright / TypeScript | MIT or Apache-2.0, as applicable | Build and validation tooling |
| Microsoft Edge WebView2 Runtime | Microsoft WebView2 distribution terms | Desktop WebView; downloaded by setup only if missing |
| Android Debug Bridge / AdbWinApi / AdbWinUsbApi | Apache-2.0 and licenses in the accompanying NOTICE | Bundled Windows USB connection component |

## Included license texts

The Windows installer includes `THIRD_PARTY_LICENSES.txt`. The main executable embeds project and dependency licenses; run it with `--licenses` to write them to the terminal or a file. Android dependency licenses are in `assets/THIRD_PARTY_LICENSES-ANDROID.txt` inside the APK.

The USB component comes from Google’s official Android Platform-Tools. Its original binaries and complete `usb/NOTICE.txt` are retained. The Android SDK installer is not bundled. [ADB source and license](https://android.googlesource.com/platform/packages/modules/adb/) · [Platform-Tools](https://developer.android.com/tools/releases/platform-tools)

## Optional audio driver

VB-CABLE is provided by VB-Audio as donationware. The Windows installer includes the original official driver archive. Users can launch its setup from **Preferences → Phone microphone**. Use and redistribution remain subject to VB-Audio’s terms; the driver is not covered by PlumeLink’s MIT license. [VB-CABLE](https://vb-cable.com/) · [VB-Audio licensing terms](https://vb-audio.com/Services/licensing.htm)

Support VB-Audio through donations or license purchases. For professional deployments where end users cannot individually identify or support VB-CABLE, distributors should purchase the corresponding licenses for their deployment count. [VB-Audio licensing information](https://vb-audio.com/Services/licensing.htm)
