# Contributing

## Issues and pull requests

Describe what you wanted to do, how to reproduce the problem and what happened. For connection issues, include both operating system versions and whether you used Wi-Fi or USB. Redact pairing codes, credentials, private input, device addresses and personal filenames from reports. [Open an issue](https://github.com/kuiwaiwai/plumelink/issues).

Discuss substantial features or protocol changes in an issue first. Keep each pull request focused, explain the resulting behavior and include relevant validation. Check user-facing text in English, Simplified Chinese and Traditional Chinese, and update the corresponding guide when behavior changes.

## Build from source

On Windows x64, install Node.js 22.12+, Rust 1.98+ with the MSVC toolchain, Visual C++ Build Tools, the Windows SDK and WebView2. Android builds also need JDK 17, Android SDK Platform 35 and Build Tools 35.0.0.

From the project root:

```powershell
npm ci
rustup target add wasm32-unknown-unknown
npm run dev
```

Run `npm run build:android` in a second terminal to build the Android client. The debug APK is written to `android-app/app/build/outputs/apk/debug/app-debug.apk`.

Windows builds download and verify the USB component pinned in `scripts/usb-component.json`, then reuse the local cache. Before invoking Cargo directly, run `node scripts/prepare-usb.mjs`.

Native Android views are in `android-app/`. Their state and communication runtime are in `apps/mobile/` and connect to the native layer through a hidden WebView. The desktop interface is in `apps/desktop-ui/`, the Windows backend in `src-tauri/` and the transport kernel in `QFastTransmission/`.

## Validation

Run the checks relevant to your changes:

| Command | Coverage |
| --- | --- |
| `npm run verify:public-source` | Public source files, private data and local documentation links |
| `npm run typecheck` | TypeScript and Vue types |
| `npm test` | Frontend and shared logic |
| `npm run test:rust` | Windows backend |
| `npm run lint:rust` | Rust Clippy checks |
| `npm run verify:mobile` | Native bridge and communication |
| `npm run verify:desktop-ui -- --connection-states` | Desktop layout and connection guidance using a simulated Tauri bridge |

Validate Android layouts, system interactions and hardware behavior on a device. Simulated bridges and audio devices do not verify Windows input or actual microphone routing.

## Packaging

Keep signing files outside the repository. Set `PLUMELINK_KEYSTORE`, `PLUMELINK_KEY_ALIAS`, `PLUMELINK_STORE_PASSWORD` and `PLUMELINK_KEY_PASSWORD` for the Android release signature, then run `npm run pack`.

The `release/` directory contains the signed APK and Windows installer with VB-CABLE. To build into a separate directory under `artifacts/candidates/`, use `npm run pack -- -Candidate`.
