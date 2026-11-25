# Microphone and file transfers

[Overview](../README.md) · [Troubleshooting](TROUBLESHOOTING.md)

## Use the phone as a microphone

Pair the devices, then open **Preferences → Phone microphone** on Windows to choose where the sound goes.

| Use | Output in PlumeLink on Windows | Microphone in the receiving app |
| --- | --- | --- |
| Listen through PC speakers or headphones | Those speakers or headphones | No input selection needed |
| Send audio into an app through VB-CABLE | `CABLE Input` | `CABLE Output` |
| Use another virtual audio cable | Its playback endpoint | The same cable’s recording endpoint |

### Start and stop

Hold the microphone button on the Remote page to talk. Releasing stops capture. To keep recording, slide up and hold for one second until the progress ring completes, then release. Stop with the on-screen button or the notification action. A short tap does not start recording.

You can also start continuous recording from **Microphone status & help**, available in the phone’s microphone settings and More menu. On first use, grant Android’s recording permission, then start again; granting permission does not start capture automatically. Opening the status panel does not record or play a test tone.

Remote control and file transfers remain available while recording. Transfers yield bandwidth to live audio and return to normal pacing when capture stops.

### Use the microphone on a website

1. On Windows, open **Preferences → Phone microphone** and run the detection and configuration action. Install VB-CABLE there if a virtual audio cable is needed.
2. Route audio as shown in the table. Allow microphone access in the website and choose the corresponding recording endpoint. If the website follows the Windows default, set that endpoint as the default input.
3. Start capture on the phone and check the website’s input level. Sound reaching the PC audio device does not mean the receiving app has started recording.

If you hear the phone through the PC speakers but the website has no input, check both endpoint selections. Stop capture before running the audio route check in **Connection diagnostics**; it plays a test tone to identify the recording endpoint.

### Volume and capture mode

The PC’s microphone gain defaults to +6 dB and can be raised to +36 dB. Increase it gradually for quiet speech and reduce it if the sound clips or becomes distorted.

In **Connection help → Connection diagnostics**, refresh the Windows device status to inspect playback volume, the confirmed recording endpoint’s volume and mute state. Check the receiving app’s input level separately. If no recording endpoint has been confirmed, check the audio route first.

While stopped, choose **Speech enhanced** in the phone’s capture-mode setting to use system automatic gain and noise suppression where available. Unsupported processing is skipped. Choose **Standard** if enhancement sounds unnatural or raises background noise; PC gain applies to either mode.

### Interruptions and recovery

- **Capture restricted:** Check Android’s microphone privacy switch and other apps using the microphone, such as calls or recorders. Stop and restart capture if it does not recover after the conflict ends.
- **Repeated disconnections:** Continuous capture retries briefly, then stops with a reason. Fix the connection or audio configuration before restarting from the status panel. This starts a new recording.
- **Remote disconnected:** Audio and remote control have separate connection states. An active microphone may still be recording; use Stop at any time. Stale status is not treated as proof that the PC received audio.
- **USB connection expired:** Reconnect the phone under **Preferences → USB connection** on Windows.

## Save PC files to the phone

Choose files on the Windows file page or drop them into its sharing area. On the phone, open **Files**, choose an **Android download folder** and grant access before saving. Removing a shared file on the PC leaves copies already saved to the phone intact.

Entering or refreshing the file page checks folder access. If the folder moved, was deleted or lost its permission, choose and authorize a folder again to resume saving.

## Send phone files to the PC

Confirm the **Windows download folder** on the PC. Choose files on the phone’s Files page, review the pending list and target PC, then send. You can also use **Share → PlumeLink** from a gallery or file manager. Files with duplicate names are saved under separate names.

Select up to 16 files at a time and remove individual entries or clear the list before sending. Sharing does not send automatically. Shares received during a transfer stay pending for later confirmation.

If reading a file fails, restore access in its source app and use the read-again action. This refreshes file information without sending it. If it still fails, select or share the file again from its source.

The Windows file page shows incoming progress and results. Use the task’s stop button to interrupt it; if saving has begun, wait for the actual result. Open a completed file’s folder to locate it, even after changing the receive folder. Clearing completed records does not delete files. PC receive history lasts only while PlumeLink is running.

## Interrupted or incomplete transfers

The limit is 2 GB per file. Restore the connection after an interruption. For an ended task, check free space and folder access before sending or saving again.

Android writes to a temporary file when the destination supports it, then assigns the final name after receiving the complete file. Cleanup only targets temporary files registered by PlumeLink and retains existing files with the same name.

If the destination cannot rename files, an unexpected exit may leave an incomplete file. Expand its details on the Files page to check its full name and location, then keep it or confirm deletion. Keeping removes the record while leaving the file in place. Deletion shows the full target and checks for changes before removing it. Restore folder access first if the destination is unavailable.

An unconfirmed save result needs a directory check before retrying: saving can finish after a timeout or cancellation request. PlumeLink does not report it as cancelled or resend it automatically. See [Transfer troubleshooting](TROUBLESHOOTING.md#file-transfer-results).
