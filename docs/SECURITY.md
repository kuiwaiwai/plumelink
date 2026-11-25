# Devices and permissions

Use PlumeLink on a trusted local network. Compare the phone’s connection address and certificate fingerprint with the PC before trusting it. Keep pairing codes private and do not forward the service port to the internet.

## Pairing and PC identity

Nearby discovery provides connection candidates. The phone checks the PC’s certificate and identity before sending pairing credentials or updating a saved address. A different IP address or port does not automatically authorize a different certificate.

If reinstalling the PC app clears its settings, its identity or certificate may change. Android asks you to verify the PC again. Cancelling preserves the original pairing; confirming removes that PC’s old credentials and requires fresh pairing. Old tokens are not sent to the new certificate, and other saved PCs are unaffected. If you did not reinstall or change the PC, cancel and check the address.

The identity endpoint exposes the service type, protocol version and device identifier. A USB endpoint also exposes a digest for verifying its current channel credential. It does not expose the credential itself, pairing data or permissions.

## USB connections

USB uses an ADB reverse channel with the same HTTPS, pairing and device permissions. Android’s USB debugging approval does not replace PlumeLink pairing. The PC opens USB endpoints only on its loopback interface and validates them separately from wireless requests.

Each USB endpoint has an independent random connection credential. Restarting the PC app or replacing the endpoint invalidates it, even if an address or port is reused. The phone verifies the channel digest, PC identity and certificate before using that credential. Expiry preserves the saved PC and its wireless address. The shared loopback address is not used as the PC’s identity.

## Device permissions

Open **Devices** on Windows to inspect paired phones and change their permissions.

| Permission | Allowed actions |
| --- | --- |
| Remote control | Move the pointer, click, send keys, list windows and switch between them |
| Administrator control | Use the device’s permitted remote-control or text actions in elevated windows; enabled by default and independently revocable |
| Text | Send text to the focused PC input field |
| Files | Save files shared by the PC and send files to its receive folder |
| Microphone | Send audio to the selected PC device and inspect configuration and this phone’s recording status |
| Shutdown | Request a shutdown; disabled by default and requires identity confirmation on the phone |

Free mode lets authorized phones share control. Primary mode assigns control to a selected phone. Revoke a phone when it should no longer have access. Refreshing the pairing code retains existing paired devices.

Microphone status queries do not disclose another phone’s name, audio level or error details; they only indicate that the channel is occupied.

## Elevated Windows applications

Administrator-control permission is enabled by default and saved per device. It can be changed while a device is offline and persists across reconnection and PC app restarts.

Opening PlumeLink manually requests Windows approval for a local elevated helper. When the app starts in the background at sign-in, it requests approval when you first open its main window. Cancelling leaves ordinary remote control available. Use **Request Windows approval** in **Devices** to request approval again.

Pausing the helper pauses administrator control for every device while preserving their permissions. The Windows lock screen and UAC confirmation screen must be handled locally on the PC.

For stored data and removal, see the [Privacy policy](../PRIVACY.md).
