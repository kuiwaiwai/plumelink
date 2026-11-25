<p align="center"><img src="docs/assets/logo.png" width="80" alt="PlumeLink"></p>

# PlumeLink

English · [简体中文](README.zh-CN.md)

Turn an Android phone into a touchpad, directional remote or microphone for your Windows PC. Transfer files in either direction over your local network or a USB data cable. No account is required.

[Download](https://github.com/kuiwaiwai/plumelink/releases) · [User guide](docs/INTERFACE.md) · [Troubleshooting](docs/TROUBLESHOOTING.md)

## At a glance

<table>
  <tr>
    <td colspan="2" align="center"><img src="docs/assets/ui/desktop-overview.png" width="820" alt="Windows overview with device status and pairing controls"><br>Windows overview</td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="docs/assets/ui/android-touchpad-portrait.png" width="300" alt="Android touchpad in portrait"><br>Touchpad</td>
    <td width="50%" align="center"><img src="docs/assets/ui/android-dpad-portrait.png" width="300" alt="Android directional remote in portrait"><br>Directional remote</td>
  </tr>
  <tr>
    <td colspan="2" align="center"><img src="docs/assets/ui/android-touchpad-landscape.png" width="820" alt="Android touchpad and controls in landscape"><br>Landscape touchpad</td>
  </tr>
</table>

## Where it comes in handy

- **PC in the study, TV in the living room?** When your PC is already connected to that display, use your phone to navigate and control playback from the sofa.
- **Already comfortable in bed?** Adjust the volume, pause a video or type on the PC without getting out from under the covers.
- **Have an old Android phone?** Give it a place beside the sofa or bed as a dedicated remote. Check the Android and WebView requirements below.

## Installation

Download both packages from [GitHub Releases](https://github.com/kuiwaiwai/plumelink/releases).

| Device | Package | Requirements |
| --- | --- | --- |
| Phone | Android APK | Android 8.0 or later; Android System WebView or Chrome as its WebView provider, version 85 or later |
| PC | Windows installer with VB-CABLE (`-setup-with-vb-cable.exe`) | Windows 10/11 x64 and Microsoft Edge WebView2 |

Install the APK on your phone and run the installer on your PC. Windows setup downloads WebView2 if it is missing. VB-CABLE is used to route the phone microphone into recording software; remote control and file transfers work without configuring the audio driver.

## Connect your phone

### Local network

1. Connect both devices to the same trusted local network. The PC can use Wi-Fi or Ethernet.
2. Open PlumeLink on the PC to display its QR code and six-digit pairing code.
3. Scan the QR code in the Android app, select a nearby PC or enter its address manually. Check the certificate fingerprint and complete pairing. The default service port is `17840`.

The phone remembers paired PCs. Manage each phone’s permissions or revoke access under **Devices** on Windows. Keep pairing codes private and use PlumeLink on a trusted network. See [Devices and permissions](docs/SECURITY.md).

### USB data cable

1. Enable USB debugging in the phone’s developer options, connect a data cable and approve the PC on the phone.
2. On Windows, open **Preferences → USB connection**, check for devices and connect your phone. The USB component is included in the installer.
3. On first use, compare the address and certificate with the PC’s USB panel, then enter its pairing code.

Remote control, text, window selection, files and microphone audio are available over USB with the same device permissions. USB tethering is not required. Reconnecting the cable restores a phone selected during the current PC session; after restarting PlumeLink on Windows, select the phone again. Its pairing is retained.

## Control your PC

| Feature | What you can do |
| --- | --- |
| Touchpad | Move, tap to click, hold to drag, scroll with two fingers and pinch to zoom |
| Directional remote | Navigate with a D-pad, confirm or cancel; hold a direction to repeat and choose a dial size in Settings |
| Keyboard and windows | Send text to the focused PC field, find a window and switch to it |
| Shortcuts and media | Configure three shortcuts with your choice of icons, key combinations or editing, browsing, presentation and media presets |
| Quick launch | Open apps and folders configured under **Preferences → Remote control settings → PC shortcuts** on Windows |

In landscape, swap the controls between sides or use focus mode to make more room. Save up to eight custom shortcut plans per PC and switch between them under **More → Shortcut plans**. System media actions work with players that support Windows media keys.

Both apps support English, Simplified Chinese and Traditional Chinese, with light, dark and system themes.

PlumeLink uses the open-source [QFastTransmission 0.2.0](https://github.com/kuiwaiwai/plumelink/tree/main/QFastTransmission) kernel to improve **consistency**, **reliability** and **stability**.

## Files and microphone

**Files:** Share files from the Windows file page, then save them from **Files** on the phone. To send files to the PC, choose them in PlumeLink or use **Share → PlumeLink** from another Android app, review the list and confirm the target PC. Choose a receive folder on each device. The limit is 2 GB per file.

**Microphone:** Configure the output under **Preferences → Phone microphone** on Windows. To use the phone as a microphone in a website or recording app, send audio to `CABLE Input` in PlumeLink and select `CABLE Output` as the microphone in that app. Hold the phone’s microphone button to talk; slide up and hold for one second to keep recording after release. Use the stop button to end it.

[Microphone and file transfer guide](docs/REMOTE-AUDIO-TRANSFERS.md)

## Documentation and contributions

[User guide](docs/INTERFACE.md) · [Troubleshooting](docs/TROUBLESHOOTING.md) · [Devices and permissions](docs/SECURITY.md) · [Privacy policy](PRIVACY.md)

Questions, bug reports and contributions are welcome through [GitHub Issues](https://github.com/kuiwaiwai/plumelink/issues) and pull requests. See [Contributing](CONTRIBUTING.md) for build requirements and commands.

PlumeLink is licensed under [MIT](LICENSE). Third-party components and the audio driver retain their own terms: [Third-party notices](THIRD_PARTY_NOTICES.md).

## Community

[LINUX DO](https://linux.do/)
