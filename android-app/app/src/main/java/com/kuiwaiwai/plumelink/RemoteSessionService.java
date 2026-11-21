package com.kuiwaiwai.plumelink;

import android.app.*;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.net.Uri;
import android.net.wifi.WifiManager;
import android.os.*;
import java.io.Closeable;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import org.json.JSONObject;

/** Owns active LAN work independently of Activity/WebView lifetime. */
public final class RemoteSessionService extends Service {
    private String ui(int id) { return AppLanguage.text(this, id); }

    private static final String CHANNEL = "remote_session";
    private static final int NOTIFICATION = 21;
    private static final String STOP_MIC = "stop_microphone", CANCEL = "cancel_transfer";
    interface Listener { void event(String name, JSONObject state); }
    final class LocalBinder extends Binder { RemoteSessionService service() { return RemoteSessionService.this; } }
    private final Handler main = new Handler(Looper.getMainLooper());
    private final LocalBinder binder = new LocalBinder();
    private final AtomicBoolean cancelled = new AtomicBoolean();
    private final AtomicReference<Closeable> active = new AtomicReference<>();
    private Listener listener;
    private MicrophoneCapture microphone;
    private volatile boolean micActive;
    private boolean locked, transferring, keepAlive, destroyed;
    private int request, generation, percent;
    private int transferGeneration;
    private long began, notificationAt;
    private int foregroundTypes;
    private String base = "", phase = "idle", micMessage = "", transferMessage = "";
    private double level;
    private PowerManager.WakeLock wake;
    private WifiManager.WifiLock wifiLock;
    private volatile Thread transferThread;
    private final Runnable renew = new Runnable() {
        @Override public void run() {
            if (micActive || transferring || keepAlive) {
                wake.acquire(120_000);
                if (wifiLock != null && !wifiLock.isHeld()) wifiLock.acquire();
                main.postDelayed(this, 60_000);
            }
        }
    };
    private static String detail(Exception error, String fallback) {
        String message = error == null ? null : error.getMessage();
        return message == null || message.isEmpty() ? fallback : message;
    }
    private void updateChannelLanguage() {
        NotificationChannel channel = new NotificationChannel(CHANNEL, ui(R.string.ui_pc_connections_and_transfers), NotificationManager.IMPORTANCE_LOW);
        channel.setDescription(ui(R.string.ui_shows_active_transfers_and_microphone_use_with_controls));
        getSystemService(NotificationManager.class).createNotificationChannel(channel);
    }
    void languageChanged() {
        updateChannelLanguage();
        if (micActive || transferring || keepAlive) updateForeground();
    }
    @Override public void onCreate() {
        super.onCreate();
        updateChannelLanguage();
        wake = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "PlumeLink:remote-session");
        wake.setReferenceCounted(false);
        WifiManager wifi = getApplicationContext().getSystemService(WifiManager.class);
        if (wifi != null) {
            int mode = Build.VERSION.SDK_INT >= 29 ? WifiManager.WIFI_MODE_FULL_LOW_LATENCY : WifiManager.WIFI_MODE_FULL_HIGH_PERF;
            wifiLock = wifi.createWifiLock(mode, "PlumeLink:remote-session");
            wifiLock.setReferenceCounted(false);
        }
    }
    @Override public IBinder onBind(Intent intent) { return binder; }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && STOP_MIC.equals(intent.getAction())) stopMicrophone();
        else if (intent != null && CANCEL.equals(intent.getAction())) cancelTransfer();
        updateForeground();
        return START_NOT_STICKY;
    }
    String activeBase() { return micActive || transferring ? base : ""; }
    void listen(Listener next) { listener = next; if (next != null) snapshot(); }
    void snapshot() { emitMic(true); emitTransfer(true); }
    void background() { if (!locked) stopMicrophone(); }
    void setConnectionKeepAlive(boolean enabled) {
        if (keepAlive == enabled) return;
        keepAlive = enabled;
        if (enabled) {
            try { startService(new Intent(this, RemoteSessionService.class)); }
            catch (RuntimeException error) { keepAlive = false; }
        }
        updateForeground();
    }
    boolean lockMicrophone(int id) {
        if (!micActive || id != request) return false;
        locked = true; emitMic(false); updateForeground();
        return true;
    }
    void stopMicrophone(int id) { if (id == request) stopMicrophone(); }
    void stopAll() { cancelTransfer(); stopMicrophone(); }
    private boolean selectBase(String next) {
        if (!activeBase().isEmpty() && !base.equals(next)) return false;
        base = next;
        return true;
    }
    void startMicrophone(String url, String host, String pin, String token, int id) {
        if (!selectBase(url)) {
            try { emit("plumelink-microphone", new JSONObject().put("id", id).put("phase", "error").put("message", ui(R.string.ui_previous_pc_tasks_are_stopping_try_again_shortly))); }
            catch (org.json.JSONException ignored) {}
            return;
        }
        generation++;
        if (microphone != null) microphone.stop();
        microphone = null; request = id; locked = false; level = 0; began = System.currentTimeMillis();
        phase = "connecting"; micMessage = ""; micActive = true;
        resumeSession = "";
        retries = 0;
        try {
            startService(new Intent(this, RemoteSessionService.class));
            updateForeground();
            emitMic(false);
            connectAudio(url, host, pin, token);
        } catch (RuntimeException error) {
            micActive = false; locked = false; phase = "error"; micMessage = ui(R.string.ui_could_not_start_background_microphone) + detail(error, ui(R.string.ui_no_details_from_the_system));
            emitMic(false); updateForeground();
        }
    }
    private String resumeSession = "";
    private int retries;
    private void connectAudio(String url, String host, String pin, String token) {
        int attempt = ++generation;
        microphone = new MicrophoneCapture(this, url, host, pin, token, resumeSession,
            (nextPhase, nextLevel, message) -> main.post(() -> {
                if (destroyed || attempt != generation) return;
                if ("confirmed".equals(nextPhase)) {
                    resumeSession = message;
                    retries = 0;
                    return;
                }
                if ("networkError".equals(nextPhase) && locked && !resumeSession.isEmpty() && retries < 2) {
                    int waiting = ++generation;
                    retries++;
                    phase = "reconnecting"; level = 0; microphone = null;
                    emitMic(false);
                    main.postDelayed(() -> {
                        if (!destroyed && waiting == generation && locked && micActive)
                            connectAudio(url, host, pin, token);
                    }, retries * 1600L);
                    return;
                }
                boolean warningChanged = !micMessage.equals(message);
                phase = "networkError".equals(nextPhase) ? "error" : nextPhase;
                level = nextLevel; micMessage = message;
                if ("error".equals(phase)) { generation++; micActive = false; locked = false; microphone = null; }
                emitMic(false);
                if (!micActive || warningChanged) updateForeground();
            }));
    }
    private void stopMicrophone() {
        generation++;
        if (microphone != null) microphone.stop();
        microphone = null; micActive = false; locked = false; phase = "idle"; level = 0; micMessage = "";
        emitMic(false); updateForeground();
    }
    interface Transfer { String run() throws Exception; }
    private void transfer(String url, String initialMessage, Transfer job) {
        if (transferring) { snapshot(); return; }
        if (!selectBase(url)) { transferMessage = ui(R.string.ui_previous_pc_tasks_are_stopping_try_again_shortly); emitTransfer(false); return; }
        transferMessage = initialMessage;
        cancelled.set(false); transferring = true; percent = 0;
        final int ownTransferGeneration = ++transferGeneration;
        try {
            startService(new Intent(this, RemoteSessionService.class));
            updateForeground(); emitTransfer(false);
        } catch (RuntimeException error) {
            transferring = false;
            transferMessage = ui(R.string.ui_could_not_start_background_transfer);
            emitTransfer(false);
            updateForeground();
            return;
        }
        transferThread = new Thread(() -> {
            String failure = null;
            String completion = null;
            try {
                completion = job.run();
            } catch (Exception error) {
                if (cancelled.get()) failure = ui(R.string.ui_transfer_canceled);
                else {
                    String reason = error instanceof TransferFailure known ? known.message(this)
                        : ui(R.string.ui_transfer_check_connection_access_storage);
                    failure = ui(R.string.ui_transfer_failed) + reason;
                }
            }
            String result = failure;
            String doneMessage = completion == null || completion.isEmpty() ? ui(R.string.ui_file_transfer_complete) : completion;
            main.post(() -> {
                if (destroyed || ownTransferGeneration != transferGeneration) return;
                transferring = false; transferThread = null;
                if (result != null) transferMessage = result;
                else { percent = 100; transferMessage = doneMessage; }
                emitTransfer(false, result == null); updateForeground();
            });
        }, "PlumeLink-file-transfer");
        transferThread.start();
    }
    private void progress(String message, int value) {
        final Thread source = Thread.currentThread();
        final String safeMessage = boundedMessage(message);
        final int safePercent = Math.max(0, Math.min(100, value));
        main.post(() -> { if (!destroyed && transferring && !cancelled.get() && source == transferThread) { transferMessage = safeMessage; percent = safePercent; emitTransfer(false);
            if (SystemClock.elapsedRealtime() - notificationAt > 1000) updateForeground();
        } });
    }
    void download(String url, String host, String pin, String path, String name, String token, String tree) {
        transfer(url, ui(R.string.ui_receiving) + name, () -> {
            ResumableDownload.save(getContentResolver(), new PinnedConnections(host, pin), url + path, token, tree,
                name, cancelled, active, value -> progress(ui(R.string.ui_receiving) + name, value));
            return ui(R.string.ui_saved) + name;
        });
    }
    void upload(String url, String host, String pin, String token, List<Uri> files) {
        transfer(url, ui(R.string.ui_preparing_to_send_files), () -> {
            PinnedConnections network = new PinnedConnections(host, pin);
            String lastName = "";
            for (int i = 0; i < files.size(); i++) {
                final String position = (i + 1) + "/" + files.size() + " · ";
                final String[] currentName = {""};
                ResumableUpload.send(getContentResolver(), network, url, token, files.get(i), cancelled, active, () -> micActive,
                    (name, value, retrying) -> {
                        currentName[0] = name;
                        progress(ui(R.string.ui_sending) + position + name + (retrying ? ui(R.string.ui_resuming) : ""), value);
                    });
                lastName = currentName[0];
            }
            if (files.size() == 1 && !lastName.isEmpty()) return ui(R.string.ui_sent) + lastName;
            return ui(R.string.ui_sent) + files.size() + ui(R.string.ui_files);
        });
    }
    void cancelTransfer() {
        if (!transferring || !cancelled.compareAndSet(false, true)) return;
        Closeable handle = active.get();
        if (handle != null) {
            new Thread(() -> {
                try {
                    handle.close();
                } catch (Exception ignored) {
                }
            }, "PlumeLink-cancel-http").start();
        }
        if (transferThread != null) transferThread.interrupt();
        transferMessage = ui(R.string.ui_canceling_transfer);
        emitTransfer(false);
    }
    private void emit(String name, JSONObject state) { if (listener != null) listener.event(name, state); }
    private void emitMic(boolean isSnapshot) {
        try { emit("plumelink-microphone", new JSONObject().put("id", request).put("phase", phase).put("level", level)
            .put("message", boundedMessage(micMessage)).put("locked", locked).put("began", began).put("snapshot", isSnapshot)); } catch (org.json.JSONException ignored) {}
    }
    private boolean transferSucceeded;
    private void emitTransfer(boolean isSnapshot) { emitTransfer(isSnapshot, isSnapshot && transferSucceeded); }
    private void emitTransfer(boolean isSnapshot, boolean succeeded) {
        if (!isSnapshot) transferSucceeded = succeeded;
        try { emit("plumelink-native-transfer", new JSONObject().put("succeeded", succeeded).put("done", !transferring).put("progress", percent)
            .put("message", boundedMessage(transferMessage)).put("snapshot", isSnapshot)); } catch (org.json.JSONException ignored) {}
    }
    private static String boundedMessage(String value) {
        if (value == null || value.isEmpty()) return "";
        return value.length() <= 512 ? value : value.substring(0, 512);
    }
    private PendingIntent action(String name) {
        return PendingIntent.getService(this, name.hashCode(), new Intent(this, RemoteSessionService.class).setAction(name), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }
    private void updateForeground() {
        main.removeCallbacks(renew);
        if (destroyed || (!micActive && !transferring && !keepAlive)) {
            if (wake.isHeld()) wake.release();
            if (wifiLock != null && wifiLock.isHeld()) wifiLock.release();
            foregroundTypes = 0;
            stopForeground(STOP_FOREGROUND_REMOVE); stopSelf(); return;
        }
        renew.run(); notificationAt = SystemClock.elapsedRealtime();
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        String text = micActive ? (locked ? ui(R.string.ui_continuous_microphone_in_use) : ui(R.string.ui_phone_microphone_in_use)) : transferring ? boundedMessage(transferMessage) : ui(R.string.ui_remote_connection_active);
        if (micActive && !micMessage.isEmpty()) text = boundedMessage(micMessage);
        if (micActive && transferring) text = boundedMessage(text + ui(R.string.ui_file_transfer) + percent + "%");
        Notification.Builder builder = new Notification.Builder(this, CHANNEL).setSmallIcon(R.drawable.ic_session)
            .setContentTitle(ui(R.string.app_name)).setContentText(text).setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true)
            .setCategory(Notification.CATEGORY_SERVICE).setVisibility(Notification.VISIBILITY_PRIVATE);
        if (micActive) builder.addAction(new Notification.Action.Builder(null, ui(R.string.ui_stop_microphone), action(STOP_MIC)).build());
        if (transferring) { builder.setProgress(100, percent, false); builder.addAction(new Notification.Action.Builder(null, ui(R.string.ui_cancel_transfer), action(CANCEL)).build()); }
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                int types = 0;
                if (transferring) types |= ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC;
                if (micActive && Build.VERSION.SDK_INT >= 30) types |= ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE;
                if (types == 0) types = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE;
                if (foregroundTypes != types) { startForeground(NOTIFICATION, builder.build(), types); foregroundTypes = types; }
                else getSystemService(NotificationManager.class).notify(NOTIFICATION, builder.build());
            } else if (foregroundTypes == 0) { startForeground(NOTIFICATION, builder.build()); foregroundTypes = 1; }
            else getSystemService(NotificationManager.class).notify(NOTIFICATION, builder.build());
        } catch (RuntimeException ignored) {
            // Avoid crashing when Android 14/15 denies background foreground-service promotion.
            getSystemService(NotificationManager.class).notify(NOTIFICATION, builder.build());
        }
    }
    @Override public void onTimeout(int startId, int fgsType) {
        stopAll();
        if (wake.isHeld()) wake.release();
        if (wifiLock != null && wifiLock.isHeld()) wifiLock.release();
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf(startId);
    }
    @Override public void onDestroy() {
        destroyed = true; listener = null; stopAll(); main.removeCallbacksAndMessages(null);
        if (wake.isHeld()) wake.release();
        if (wifiLock != null && wifiLock.isHeld()) wifiLock.release();
        super.onDestroy();
    }
}
