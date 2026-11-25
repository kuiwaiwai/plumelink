# Troubleshooting

[Overview](../README.md) · [Devices and permissions](SECURITY.md) · [Microphone and files](REMOTE-AUDIO-TRANSFERS.md)

## USB connection

The Windows installer includes the USB component. Open **Preferences → USB connection** to detect and connect a phone. If connection types are unavailable, choose the phone attached by a data cable; the list may also contain wireless devices or emulators.

| What you see | What to try |
| --- | --- |
| Component missing or unable to run | Reinstall PlumeLink on Windows and check whether security software quarantined files from its installation directory |
| Background service cannot start | Check security software; close other phone-management tools or emulators and detect again |
| Background service does not respond or is incompatible | Exit programs using another ADB version and detect again; restart the PC if needed. PlumeLink does not force-stop a shared ADB service |
| No phone found | Try a data-capable cable and another USB port, enable USB debugging and install the phone manufacturer’s USB driver if Windows does not recognize it |
| Waiting for authorization | Unlock the phone and approve this PC in Android’s USB debugging prompt |
| PlumeLink is not open on the phone | Install the Android app, unlock the phone and connect again |
| USB channel ready, but not paired | Complete certificate confirmation and pairing on the phone. The channel alone does not grant access; check authorization under Devices on the PC |
| USB connection expired | Connect the phone again in the PC’s USB panel to update its endpoint. Saved pairing and the wireless address are retained |
| USB endpoint occupied | Disconnect other USB remote-control tools, then retry. PlumeLink does not overwrite their channels |
| Cable disconnected | Reconnect and confirm Android’s authorization. The PC restores phones selected in its current session; after restarting the app, select the phone again |
| Cleanup incomplete | Automatic recovery for that phone is cancelled. Keep PlumeLink running, reconnect the cable and detect again so cleanup can finish |
| Connection result unconfirmed | Detect again, then connect that phone. PlumeLink retires the uncertain endpoint and uses a fresh one without interrupting other phones |

For multiple phones, each address under a phone belongs to its own USB endpoint. Compare that address during certificate verification. Disconnecting one phone leaves the others available; stopping all services disconnects every session. An offline phone can be disconnected individually to cancel its automatic recovery.

USB does not require shared Wi-Fi or USB tethering. To use Wi-Fi instead, choose the wireless action from the saved PC’s menu on the phone. Microphone output still needs configuration under **Preferences → Phone microphone** on Windows.

For component diagnostics, run `PlumeLink.exe --verify-usb-connection` from a terminal in its installation directory. The output distinguishes component, background-service and device-list status without printing phone serial numbers or pairing data. A failed check alone does not mean an active channel has disconnected.

## Cannot find or reach the PC

1. Open **Connection diagnostics** on Windows. Check that the service is running and the PC is awake.
2. Connect both devices to the same trusted local network. The PC can use Ethernet. Avoid guest Wi-Fi that prevents devices from reaching one another.
3. Scan the PC’s QR code or enter its displayed address and port manually. The default port is `17840`. Manual connection works even when discovery is unavailable.

If connection still times out, check the selected Wi-Fi or Ethernet adapter on the PC. Allow PlumeLink through Windows Firewall on your trusted network. If another application uses the port, change it under **Preferences → Connection settings**, apply successfully and scan again.

After changing installation directories, run the installer as administrator to refresh the firewall rule if needed. The rule targets `PlumeLink.exe` in that directory and permits local-subnet TCP traffic on private or domain networks.

When discovery finds a new address for a saved PC, select it to verify the certificate and identity before updating the address. Only a matching identity and certificate retain the pairing and shortcuts. Scanning again also works. Losing a discovery result does not delete the saved pairing.

For a paired wireless PC stuck reconnecting, tap its status on the Remote page and choose **Find and reconnect**. Stop recording and finish file transfers first. PlumeLink looks for the same PC and checks its saved certificate before updating the connection. Unsent text stays as a draft; previous remote input is not replayed. If it cannot find the PC, retry the address or return to the PC list. Use the USB panel for wired connections.

## Pairing code or certificate warning

Pairing codes refresh every 15 minutes by default and after successful pairing. Enter the six digits currently displayed on the PC. After repeated incorrect attempts, wait a minute before retrying. The interval can be changed in connection settings.

Reinstalling the PC app and clearing its settings may change its certificate or identity. Android asks you to compare the address and fingerprint with the PC and pair again. You do not need to reset trust for other PCs. Cancelling preserves the original pairing. Ordinary updates or network changes generally retain the certificate; if no reinstall or PC change was expected, cancel and check the address.

## Connected, but controls do not respond

| Problem | Check |
| --- | --- |
| Pointer or keys do nothing | Remote-control permission under Devices on Windows, and which phone has control in primary mode |
| Text does not appear | Text permission and whether the intended input field is focused on the PC |
| Only elevated windows ignore input | Administrator-control permission, the required remote or text permission, and Windows approval for the helper. See [Devices and permissions](SECURITY.md#elevated-windows-applications) |
| Playback buttons do nothing | Focus the player. The remote’s play button sends Space and its seek buttons send arrow keys; the player must support them. System media actions require Windows media-key support |

## Connection stops in the background

**Keep app running** in the phone’s display settings is on by default. While connected, a native foreground service maintains the connection and shows a notification. Turning it off pauses remote control when leaving the app. Touches and keys release on leaving; returning starts a fresh session without replaying old input.

If the connection stops after a longer screen-off period, open **Background settings**, allow background use and choose an unrestricted battery policy. Some phones also require autostart permission. Deep power saving, force-stopping the app or loss of network access can interrupt the connection. Keep both apps up to date.

Locked continuous capture and active file transfers can continue in the background when Android permits them. Check microphone, notification and background permissions. If the foreground service could not start, remove the system restriction and toggle keep-alive off and on. Return to PlumeLink to restart a task that stopped. If the PC revoked file or microphone access, enable that device’s permission on Windows before retrying; pairing can be retained.

You can return to the PC list and choose another computer even if the previous one is offline.

## Quick launch fails or has no confirmed result

If an app or folder moved or was deleted, update it under **Preferences → Remote control settings → PC shortcuts** on Windows and refresh the phone’s list. An unconfirmed result means you should check whether it opened on the PC before retrying; the launch is not automatically repeated.

You can close the sheet while a launch is pending. Another pending request may require waiting. Requests cancelled or timed out during preparation do not run later; a request already handed to Windows may need its result checked. A failed status check does not prevent refreshing the list or require restarting the app.

## PC wakes from sleep

The phone starts a fresh remote session and retains pairing. Earlier touches, keys and queued actions are discarded. Wait for the connection before controlling the PC again. For USB, detect and reconnect the phone in the PC’s USB panel if needed.

## File transfer results

**Cancellation result unconfirmed:** A file may already be finishing its save. PlumeLink briefly checks the result, retaining completed files and cancelling files not yet saved. If the connection prevents confirmation, inspect the receive directory before resending. The next file in a batch is not sent automatically.

**Receive folder does not respond:** Cancel the task and choose another location, such as internal storage. An unavailable cloud provider, memory card or file manager may still be processing the original operation; wait before retrying rather than starting it repeatedly.

**Save result unconfirmed:** Check the selected directory before sending or saving again. A write can finish after the timeout message. PlumeLink does not treat this as confirmed cancellation or automatically resend the file.
