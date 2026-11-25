package com.kuiwaiwai.plumelink;

import android.app.*;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.net.Uri;
import android.net.wifi.WifiManager;
import android.os.*;
import java.util.List;
import java.util.Map;
import java.util.LinkedHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
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
    private TransferTask currentTransfer;
    private Listener listener;
    private MicrophoneCapture microphone;
    private volatile boolean micActive;
    private boolean locked, transferring, keepAlive, destroyed;
    private volatile boolean controlForeground = true;
    private boolean keepAliveFailed;
    private int controlTerminalCode;
    private String selectedPin = "", controlToken = "", backgroundPhase = "idle";
    private volatile Object controlHost;
    private static final class KeepAliveSnapshot {
        final String token, owner, transport, json;
        KeepAliveSnapshot(String token, String owner, String transport, String json) {
            this.token = token; this.owner = owner; this.transport = transport; this.json = json;
        }
    }
    private volatile KeepAliveSnapshot keepAliveSnapshot = new KeepAliveSnapshot("", "", "", "{}");
    private BackgroundControlConnection backgroundControl;
    private okhttp3.OkHttpClient controlClient;
    private int request, generation, percent;
    private int transferGeneration;
    private long began, notificationAt;
    private int foregroundTypes;
    private String base = "", phase = "idle", micMessage = "", transferMessage = "", transferBase = "";
    private String selectedBase = "", selectedIdentity = "", selectedOwner = "", baseOwner = "", transferOwner = "";
    private String selectedUsbAccess = "", selectedTransport = "", baseTransport = "", transferTransport = "";
    private String transferDirection = "", transferName = "";
    private int transferCount;
    private double level;
    private PowerManager.WakeLock wake;
    private WifiManager.WifiLock wifiLock;
    private volatile Thread transferThread;
    private TransferJournal transferJournal;
    private boolean recoveredTransfer;
    private String recoveredTransferId = "";
    private PendingDownloads pendingDownloads;
    private final Map<String,Object> recoveryTasks = new LinkedHashMap<>();
    private final Map<String,Thread> recoveryThreads = new LinkedHashMap<>();
    private final Map<String,List<PendingDownloads.Recovered>> recoveredBySource = new LinkedHashMap<>();
    private final Map<String, org.json.JSONArray> unfinishedDownloads = new LinkedHashMap<>();
    private String recoveryMessage = "";
    private TransferFailure recoveryFailure;
    private final Runnable renew = new Runnable() {
        @Override public void run() {
            if (micActive || transferring || keepAlive) {
                wake.acquire(120_000);
                if (!EndpointUtils.isUsbEndpoint(selectedBase) && wifiLock != null && !wifiLock.isHeld()) wifiLock.acquire();
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
        if (recoveryFailure != null) { recoveryMessage = recoveryFailure.message(this); emitRecovery(); }
        if (recoveredTransfer) { transferMessage = ui(R.string.ui_previous_transfer_interrupted); emitTransfer(true); }
        updateChannelLanguage();
        if (micActive || transferring || keepAlive) updateForeground();
        emitKeepAlive();
    }
    @Override public void onCreate() {
        super.onCreate();
        transferJournal = new TransferJournal(getFilesDir());
        pendingDownloads = new PendingDownloads(getFilesDir());
        TransferJournal.Entry interrupted = transferJournal.recover();
        if (interrupted != null) {
            recoveredTransfer = true;
            recoveredTransferId = interrupted.id;
            transferBase = interrupted.origin;
            transferOwner = interrupted.owner;
            transferMessage = ui(R.string.ui_previous_transfer_interrupted);
        }
        updateChannelLanguage();
        wake = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "PlumeLink:remote-session");
        wake.setReferenceCounted(false);
        backgroundControl = new BackgroundControlConnection(new BackgroundControlConnection.Scheduler() {
            public long now() { return SystemClock.elapsedRealtime(); }
            public void post(Runnable action, long delay) { main.postDelayed(action, delay); }
            public void remove(Runnable action) { main.removeCallbacks(action); }
        }, (state, code) -> {
            backgroundPhase = state;
            controlTerminalCode = code;
            if (code != 0) { keepAlive = false; updateForeground(); }
            emitKeepAlive();
        });
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
    String activeBase() { return micActive || (transferring && currentTransfer != null && !currentTransfer.cancelled.get()) ? base : ""; }
    boolean hasActiveWork() { return micActive || transferring; }
    String computerIdentity() { return selectedIdentity; }
    String usbAccess() { return selectedUsbAccess; }
    String transportIdentity() { return selectedTransport; }
    void selectComputer(String url, String identity, String pin) {
        selectComputer(url, identity, pin, new ComputerStore(this).usbAccess(url));
    }
    void selectComputer(String url, String identity, String pin, String usbKey) {
        String access = EndpointUtils.isUsbEndpoint(url) && UsbAccess.validKey(usbKey) ? usbKey : "";
        String owner = SessionIdentity.owner(url,identity,pin);
        String transport = SessionIdentity.transport(url,owner,access);
        if (!selectedBase.equals(url) || !selectedOwner.equals(owner) || !selectedTransport.equals(transport)) {
            stopAll(); selectedBase = url; selectedIdentity = identity; selectedOwner = owner;
            selectedPin = pin;
            selectedUsbAccess = access; selectedTransport = transport;
            if (EndpointUtils.isUsbEndpoint(url) && wifiLock != null && wifiLock.isHeld()) wifiLock.release();
            recoveryMessage = ""; recoveryFailure = null; emitRecovery();
        }
    }
    void listen(Listener next) { listener = next; if (next != null) snapshot(); }
    void snapshot() {
        emitMic(true); emitTransfer(true);
        emitKeepAlive();
        emitRecovery();
        refreshDownloads();
    }
    private void emitRecovery() {
        try { emit("plumelink-download-recovery", new JSONObject().put("files", recoveryFiles()).put("message", recoveryMessage)); }
        catch (org.json.JSONException ignored) {}
    }
    private org.json.JSONArray recoveryFiles() {
        org.json.JSONArray files = new org.json.JSONArray();
        java.util.ArrayList<org.json.JSONArray> groups = new java.util.ArrayList<>();
        org.json.JSONArray owned = unfinishedDownloads.get(selectedOwner);
        if (owned != null) groups.add(owned);
        if (!EndpointUtils.isUsbEndpoint(selectedBase)) {
            org.json.JSONArray legacy = unfinishedDownloads.get("legacy:" + selectedBase); if (legacy != null) groups.add(legacy);
        }
        for (org.json.JSONArray group : groups)
            for (int i = 0; i < group.length() && files.length() < 64; i++) files.put(group.optJSONObject(i));
        return files;
    }
    private void mergeRecovery() {
        unfinishedDownloads.clear();
        for (List<PendingDownloads.Recovered> source : recoveredBySource.values()) for (PendingDownloads.Recovered file : source) {
            String owner = file.entry.owner.isEmpty() ? "legacy:"+file.entry.origin : file.entry.owner;
            org.json.JSONArray items = unfinishedDownloads.computeIfAbsent(owner,ignored -> new org.json.JSONArray());
            try { if (items.length()<64) items.put(file.json()); } catch (org.json.JSONException ignored) {}
        }
        emitRecovery();
    }
    void refreshDownloads() {
        if (destroyed) return;
        Map<String,List<PendingDownloads.Entry>> groups=new LinkedHashMap<>();
        for (PendingDownloads.Entry entry : pendingDownloads.entries())
            groups.computeIfAbsent(ProviderTasks.authority(entry.tree),ignored -> new java.util.ArrayList<>()).add(entry);
        if (recoveredBySource.keySet().removeIf(key -> !groups.containsKey(key) && !recoveryTasks.containsKey(key))) mergeRecovery();
        for (Map.Entry<String,List<PendingDownloads.Entry>> group : groups.entrySet()) {
            String authority=group.getKey();
            if (recoveryTasks.containsKey(authority)) continue;
            ProviderTasks.Lease worker=ProviderTasks.SHARED.tryAcquire(java.util.List.of(authority));
            if (worker==null) continue;
            Object ticket=new Object(); recoveryTasks.put(authority,ticket);
            Thread thread=new Thread(() -> {
                List<PendingDownloads.Recovered> result;
                try (worker) {
                    result=pendingDownloads.recoverEntries(getContentResolver(),group.getValue());
                    ReceiveDirectory.releaseUnused(getContentResolver(),new ComputerStore(this)::receiveTree,pendingDownloads);
                } catch (Exception ignored) { result=null; }
                List<PendingDownloads.Recovered> completed=result;
                main.post(() -> {
                    if (destroyed || recoveryTasks.get(authority)!=ticket) return;
                    recoveryTasks.remove(authority); recoveryThreads.remove(authority);
                    if (completed!=null) recoveredBySource.put(authority,completed);
                    mergeRecovery();
                });
            },"PlumeLink-file-recovery");
            recoveryThreads.put(authority,thread);
            try { thread.start(); }
            catch (RuntimeException error) { worker.close(); recoveryTasks.remove(authority); recoveryThreads.remove(authority); }
        }
    }
    void resolveDownload(String id, String version, boolean keep, String request) {
        if (destroyed) return;
        final String taskOwner=selectedOwner, taskEndpoint=selectedBase, taskTransport=selectedTransport;
        if (!pendingDownloads.belongsTo(id,taskOwner,taskEndpoint)) return;
        String authority=ProviderTasks.authority(pendingDownloads.treeFor(id));
        ProviderTasks.Lease worker=recoveryTasks.containsKey(authority) ? null : ProviderTasks.SHARED.tryAcquire(java.util.List.of(authority));
        if (worker==null) { finishRecoveryRequest(request,taskOwner,taskTransport,new TransferFailure(R.string.ui_file_provider_busy)); return; }
        recoveryMessage=""; recoveryFailure=null;
        Object ticket=new Object(); recoveryTasks.put(authority,ticket);
        AtomicBoolean expired=new AtomicBoolean();
        main.postDelayed(() -> {
            if (destroyed || recoveryTasks.get(authority)!=ticket) return;
            expired.set(true);
            finishRecoveryRequest(request,taskOwner,taskTransport,new TransferFailure(R.string.ui_download_result_unconfirmed));
        },10_000);
        Thread thread=new Thread(() -> {
            TransferFailure failure=null; String acknowledged="";
            try (worker) {
                PendingDownloads.Entry changed=pendingDownloads.change(getContentResolver(),id,version,keep);
                if (changed!=null && transferJournal.acknowledge(changed.task)) acknowledged=changed.task;
            } catch (Exception error) { failure=error instanceof TransferFailure known ? known : new TransferFailure(R.string.ui_unfinished_download_remove_failed); }
            TransferFailure result=failure; String completedTask=acknowledged;
            main.post(() -> {
                if (destroyed || recoveryTasks.get(authority)!=ticket) return;
                recoveryTasks.remove(authority); recoveryThreads.remove(authority);
                if (selectedOwner.equals(taskOwner) && selectedTransport.equals(taskTransport)) {
                    if (recoveredTransfer && !transferring && !completedTask.isEmpty() && completedTask.equals(recoveredTransferId)) {
                        recoveredTransfer=false; transferMessage=""; emitTransfer(false);
                    }
                    if (!expired.get()) finishRecoveryRequest(request,taskOwner,taskTransport,result);
                }
                refreshDownloads();
            });
        },"PlumeLink-file-recovery-action");
        recoveryThreads.put(authority,thread);
        try { thread.start(); }
        catch (RuntimeException error) {
            worker.close(); recoveryTasks.remove(authority); recoveryThreads.remove(authority);
            finishRecoveryRequest(request,taskOwner,taskTransport,new TransferFailure(R.string.ui_file_provider_busy));
        }
    }
    private void finishRecoveryRequest(String request,String owner,String transport,TransferFailure result) {
        if (destroyed || !selectedOwner.equals(owner) || !selectedTransport.equals(transport)) return;
        recoveryFailure=result; recoveryMessage=result==null ? "" : result.message(this);
        try { emit("plumelink-download-recovery",new JSONObject().put("files",recoveryFiles()).put("message",recoveryMessage).put("request",request)); }
        catch (org.json.JSONException ignored) {}
    }
    boolean attachControlHost(Object host, boolean foreground) {
        if (!foreground && controlHost != null && controlHost != host) return false;
        controlHost = host;
        if (foreground) foreground(host);
        return true;
    }
    boolean isControlHost(Object host) { return controlHost == host && controlForeground; }
    boolean detachControlHost(Object host) {
        if (host != controlHost) return false;
        background(host);
        controlHost = null;
        return true;
    }
    void foreground(Object host) {
        if (host != controlHost) return;
        controlForeground = true;
        stopBackgroundConnection();
        if (controlTerminalCode == 4001) controlTerminalCode = 0;
    }
    void background(Object host) {
        if (host != controlHost) return;
        if (micActive && !locked) stopMicrophone();
        controlForeground = false;
        startBackgroundConnection();
    }
    void setControlCredentials(String token) {
        String next = token != null && token.matches("[a-f0-9]{64}") ? token : "";
        if (controlToken.equals(next)) return;
        stopBackgroundConnection();
        controlToken = next;
        controlTerminalCode = 0;
        emitKeepAlive();
        startBackgroundConnection();
    }
    void retryConnectionKeepAlive() {
        if (!controlForeground) return;
        controlTerminalCode = 0;
        keepAliveFailed = false;
        emitKeepAlive();
    }
    String connectionKeepAliveState(String token, String owner, String transport) {
        KeepAliveSnapshot state = keepAliveSnapshot;
        return state.token.equals(token) && state.owner.equals(owner) && state.transport.equals(transport) ? state.json : "{}";
    }
    private void emitKeepAlive() {
        try {
            JSONObject state = new JSONObject().put("enabled", keepAlive).put("failed", keepAliveFailed)
                .put("phase", backgroundPhase).put("terminalCode", controlTerminalCode);
            state.put("owner", selectedOwner).put("transport", selectedTransport);
            keepAliveSnapshot = new KeepAliveSnapshot(controlToken, selectedOwner, selectedTransport, state.toString());
            emit("plumelink-keepalive", state);
        } catch (org.json.JSONException ignored) {}
    }
    private void stopBackgroundConnection() {
        if (backgroundControl != null) backgroundControl.stop();
        backgroundPhase = "idle";
    }
    private void startBackgroundConnection() {
        if (destroyed || controlForeground || !keepAlive || backgroundControl == null || controlToken.isEmpty()
                || !"idle".equals(backgroundPhase) || controlTerminalCode != 0) return;
        try {
            if (controlClient == null) controlClient = new PinnedConnections(Uri.parse(selectedBase).getHost(), selectedPin, selectedBase, selectedUsbAccess).audioClient();
            final boolean usb = EndpointUtils.isUsbEndpoint(selectedBase);
            okhttp3.Request request = new okhttp3.Request.Builder().url(selectedBase + "/ws").header("Origin", selectedBase).build();
            backgroundControl.start(callbacks -> {
                okhttp3.WebSocket socket = controlClient.newWebSocket(request, new okhttp3.WebSocketListener() {
                    @Override public void onOpen(okhttp3.WebSocket ws, okhttp3.Response response) { main.post(callbacks::opened); }
                    @Override public void onMessage(okhttp3.WebSocket ws, String text) { main.post(() -> callbacks.message(text)); }
                    @Override public void onMessage(okhttp3.WebSocket ws, okio.ByteString bytes) { main.post(() -> callbacks.closed(4002)); }
                    @Override public void onClosing(okhttp3.WebSocket ws, int code, String reason) { main.post(() -> callbacks.closed(code)); }
                    @Override public void onClosed(okhttp3.WebSocket ws, int code, String reason) { main.post(() -> callbacks.closed(code)); }
                    @Override public void onFailure(okhttp3.WebSocket ws, Throwable error, okhttp3.Response response) {
                        int code = response != null && response.code() == 410 && usb ? 4409
                            : response != null && response.code() == 401 ? 4003 : 1006;
                        main.post(() -> callbacks.closed(code));
                    }
                });
                return new BackgroundControlConnection.Connection() {
                    public boolean send(String text) { return socket.send(text); }
                    public void cancel() { socket.cancel(); }
                };
            }, controlToken);
        } catch (Exception error) {
            keepAlive = false;
            keepAliveFailed = true;
            stopBackgroundConnection();
            updateForeground();
            emitKeepAlive();
        }
    }
    void setConnectionKeepAlive(boolean enabled) {
        if (enabled && controlTerminalCode != 0) return;
        if (keepAlive == enabled) return;
        keepAlive = enabled;
        keepAliveFailed = false;
        if (!enabled) stopBackgroundConnection();
        if (enabled) {
            try { startService(new Intent(this, RemoteSessionService.class)); }
            catch (RuntimeException error) { keepAlive = false; keepAliveFailed = true; }
        }
        updateForeground();
        emitKeepAlive();
        startBackgroundConnection();
    }
    boolean lockMicrophone(int id) {
        if (!micActive || id != request) return false;
        locked = true; emitMic(false); updateForeground();
        return true;
    }
    void stopMicrophone(int id) { if (id == request) stopMicrophone(); }
    void stopAll() {
        stopBackgroundConnection(); controlToken = ""; controlTerminalCode = 0;
        if (controlClient != null) {
            controlClient.dispatcher().executorService().shutdown(); controlClient.connectionPool().evictAll(); controlClient = null;
        }
        cancelTransfer(); stopMicrophone();
    }
    private boolean selectBase(String next) {
        if (selectedBase.isEmpty()) {
            ComputerStore store = new ComputerStore(this); String id = store.storedIdentityOrEmpty(next);
            selectComputer(next,id,store.trustedPin(Uri.parse(next).getHost(),id));
        }
        boolean activeTransfer=transferring && currentTransfer!=null && !currentTransfer.cancelled.get();
        if (!selectedBase.equals(next) || ((micActive || activeTransfer) && (!base.equals(next) || !baseOwner.equals(selectedOwner) || !baseTransport.equals(selectedTransport)))) return false;
        base = next;
        baseOwner = selectedOwner;
        baseTransport = selectedTransport;
        return true;
    }
    void startMicrophone(String url, String host, String pin, String token, int id) {
        if (!selectBase(url)) {
            try { emit("plumelink-microphone", new JSONObject().put("owner",selectedOwner).put("transport",selectedTransport).put("id", id).put("phase", "error").put("message", ui(R.string.ui_previous_pc_tasks_are_stopping_try_again_shortly))); }
            catch (org.json.JSONException ignored) {}
            return;
        }
        generation++;
        if (microphone != null) microphone.stop();
        microphone = null; request = id; locked = false; level = 0; began = System.currentTimeMillis();
        phase = "connecting"; micMessage = ""; micActive = true;
        resumeSession = "";
        reconnectBudget.reset();
        speechProcessing = new ComputerStore(this).speechProcessing(selectedIdentity.isEmpty() ? url : selectedIdentity);
        try {
            startService(new Intent(this, RemoteSessionService.class));
            if (!updateForeground()) {
                failMicrophonePromotion();
                return;
            }
            emitMic(false);
            connectAudio(url, host, pin, selectedUsbAccess, token);
        } catch (RuntimeException error) {
            micActive = false; locked = false; phase = "error"; micMessage = ui(R.string.ui_could_not_start_background_microphone) + detail(error, ui(R.string.ui_no_details_from_the_system));
            emitMic(false); updateForeground();
        }
    }
    private void failMicrophonePromotion() {
        generation++;
        if (microphone != null) microphone.stop();
        microphone = null; micActive = false; locked = false; level = 0; phase = "error";
        micMessage = ui(R.string.ui_microphone_background_denied);
        emitMic(false); updateForeground();
    }
    private String resumeSession = "";
    private final AudioReconnectBudget reconnectBudget = new AudioReconnectBudget();
    private boolean speechProcessing;
    private void connectAudio(String url, String host, String pin, String usbKey, String token) {
        reconnectBudget.beginAttempt();
        int attempt = ++generation;
        final String attemptOwner = baseOwner, attemptTransport = baseTransport;
        microphone = new MicrophoneCapture(this, url, host, pin, usbKey, token, resumeSession, speechProcessing,
            (nextPhase, nextLevel, message) -> main.post(() -> {
                if (destroyed || attempt != generation || !selectedTransport.equals(attemptTransport)) return;
                if ("confirmed".equals(nextPhase)) {
                    resumeSession = message;
                    reconnectBudget.confirm();
                    return;
                }
                int retryDelay = "networkError".equals(nextPhase) && locked && !resumeSession.isEmpty()
                    ? reconnectBudget.retryDelayMillis() : 0;
                if (retryDelay > 0) {
                    int waiting = ++generation;
                    phase = "reconnecting"; level = 0; microphone = null; micMessage = "";
                    emitMic(false, attemptOwner, attemptTransport);
                    updateForeground();
                    main.postDelayed(() -> {
                        if (!destroyed && waiting == generation && locked && micActive)
                            connectAudio(url, host, pin, usbKey, token);
                    }, retryDelay);
                    return;
                }
                boolean warningChanged = !micMessage.equals(message);
                boolean phaseChanged = !phase.equals(nextPhase);
                phase = "networkError".equals(nextPhase) ? "error" : nextPhase;
                if ("live".equals(phase)) reconnectBudget.live(SystemClock.elapsedRealtime());
                level = nextLevel; micMessage = message;
                if ("error".equals(phase)) { generation++; micActive = false; locked = false; microphone = null; }
                emitMic(false, attemptOwner, attemptTransport);
                if (!micActive || warningChanged || phaseChanged) updateForeground();
            }));
    }
    private void stopMicrophone() {
        generation++;
        reconnectBudget.reset();
        if (microphone != null) microphone.stop();
        microphone = null; micActive = false; locked = false; phase = "idle"; level = 0; micMessage = "";
        emitMic(false); updateForeground();
    }
    private int completedUploads, totalUploads;
    interface Transfer { String run(TransferJournal.Lease lease, TransferTask task) throws Exception; }
    private boolean transfer(String url, String initialMessage, String tree, boolean upload, java.util.Collection<String> authorities, Transfer job) {
        if (transferring) {
            if (transferBase.equals(url) && transferOwner.equals(selectedOwner) && transferTransport.equals(selectedTransport)) snapshot();
            else refuseTransfer(url);
            return false;
        }
        ProviderTasks.Lease worker = ProviderTasks.SHARED.tryAcquire(authorities);
        if (worker == null) { providerUnavailable(url); return false; }
        if (!selectBase(url)) { worker.close(); refuseTransfer(url); return false; }
        ReceiveDirectory.Access access = ReceiveDirectory.retain(tree);
        TransferTask task = new TransferTask(url,selectedOwner,selectedTransport,upload);
        currentTransfer = task;
        transferBase = url; transferOwner = task.owner; transferTransport = task.transport;
        recoveredTransfer = false;
        transferDirection = ""; transferName = ""; transferCount = 0;
        completedUploads = 0; totalUploads = 0;
        transferMessage = initialMessage;
        transferring = true; percent = 0;
        ++transferGeneration;
        try {
            startService(new Intent(this, RemoteSessionService.class));
            if (!updateForeground()) throw new IllegalStateException("Foreground transfer unavailable");
            emitTransfer(false);
        } catch (RuntimeException error) {
            access.close(); worker.close(); currentTransfer = null; transferring = false;
            transferMessage = ui(R.string.ui_could_not_start_background_transfer);
            emitTransfer(false); updateForeground(); return false;
        }
        task.thread = new Thread(() -> {
            String failure = null, completion = null;
            try (worker; access; TransferJournal.Lease lease = transferJournal.begin(url,task.owner)) {
                task.check(); completion = job.run(lease,task);
            } catch (Exception error) {
                if (error instanceof ResumableDownload.Unconfirmed unknown) {
                    failure = unknown.message(this);
                } else if (error instanceof ResumableUpload.Unconfirmed unknown) {
                    failure = unknown.message(this);
                    if (task.completed > 0) failure = AppLanguage.quantity(this, R.plurals.ui_upload_partial_unconfirmed, task.total, task.completed, task.total, failure);
                } else if (task.cancelled.get()) {
                    failure = task.completed > 0
                        ? AppLanguage.quantity(this, R.plurals.ui_upload_cancelled_partial, task.total, task.completed, task.total)
                        : ui(R.string.ui_transfer_canceled);
                } else {
                    String reason = error instanceof TransferFailure known ? known.message(this) : ui(R.string.ui_transfer_check_connection_access_storage);
                    failure = ui(R.string.ui_transfer_failed) + reason;
                    if (task.completed > 0 && task.total > 1) failure = AppLanguage.quantity(this, R.plurals.ui_batch_partially_sent, task.total, task.completed, task.total, failure);
                }
            }
            String result = failure;
            String doneMessage = completion == null || completion.isEmpty() ? ui(R.string.ui_file_transfer_complete) : completion;
            main.post(() -> {
                if (destroyed) return;
                if (currentTransfer != task) { refreshDownloads(); return; }
                copyTask(task); currentTransfer = null; transferring = false; transferThread = null;
                if (result != null) transferMessage = result;
                else { percent = 100; transferMessage = doneMessage; }
                emitTransfer(false, result == null, task.owner, task.transport); updateForeground(); refreshDownloads();
            });
        }, "PlumeLink-file-transfer");
        transferThread = task.thread;
        try { task.thread.start(); }
        catch (RuntimeException error) {
            access.close(); worker.close(); currentTransfer = null; transferring = false; transferThread = null;
            transferMessage = ui(R.string.ui_could_not_start_background_transfer); emitTransfer(false); updateForeground(); return false;
        }
        watchTransfer(task);
        return true;
    }
    private void providerUnavailable(String url) {
        try { emit("plumelink-native-transfer",new JSONObject().put("owner",selectedOwner).put("transport",selectedTransport)
            .put("origin",url).put("done",true).put("succeeded",false).put("message",ui(R.string.ui_file_provider_busy))); }
        catch (org.json.JSONException ignored) {}
    }
    private void copyTask(TransferTask task) {
        completedUploads = task.completed; totalUploads = task.total;
        transferDirection = task.direction; transferName = task.name; transferCount = task.count;
    }
    private void watchTransfer(TransferTask task) {
        main.postDelayed(() -> {
            if (destroyed || currentTransfer != task) return;
            if (!task.cancelled.get() && task.providerTimedOut()) {
                task.cancel(); transferMessage = ui(R.string.ui_file_provider_slow); emitTransfer(false);
            }
            if (task.cancellationExpired()) {
                copyTask(task); currentTransfer = null; transferring = false; transferThread = null;
                boolean known=task.upload && !task.uploadPending;
                boolean completed=known && task.total>0 && task.completed==task.total;
                if (completed) transferMessage=AppLanguage.quantity(this,R.plurals.ui_upload_cancel_completed,task.completed,task.completed);
                else if (known && task.completed>0) transferMessage=AppLanguage.quantity(this,R.plurals.ui_upload_cancelled_partial,task.total,task.completed,task.total);
                else if (known) transferMessage=ui(R.string.ui_file_provider_stopped);
                else {
                    transferMessage=ui(task.upload ? R.string.ui_upload_result_unconfirmed : R.string.ui_download_result_unconfirmed);
                    if (task.completed>0) transferMessage=AppLanguage.quantity(this,R.plurals.ui_upload_partial_unconfirmed,task.total,task.completed,task.total,transferMessage);
                }
                emitTransfer(false,completed,task.owner,task.transport); updateForeground(); refreshDownloads();
                return;
            }
            watchTransfer(task);
        },250);
    }
    private void progress(TransferTask task, String message, int value) {
        final String safeMessage = boundedMessage(message);
        final int safePercent = Math.max(0, Math.min(100, value));
        main.post(() -> {
            if (!destroyed && currentTransfer == task && transferring && !task.cancelled.get()) {
                copyTask(task); transferMessage = safeMessage; percent = safePercent; emitTransfer(false,false,task.owner,task.transport);
                if (SystemClock.elapsedRealtime() - notificationAt > 1000) updateForeground();
            }
        });
    }
    void download(String url, String host, String pin, String path, String name, String token, String tree) {
        final String usbKey = selectedBase.isEmpty() ? new ComputerStore(this).usbAccess(url) : selectedUsbAccess;
        transfer(url,ui(R.string.ui_receiving)+name,tree,false,java.util.List.of(ProviderTasks.authority(tree)),(lease,task) -> {
            ResumableDownload.save(getContentResolver(),new PinnedConnections(host,pin,url,usbKey),url+path,token,tree,
                name,task.cancelled,task.active,value -> progress(task,ui(R.string.ui_receiving)+name,value),pendingDownloads,url,lease.id(),lease.owner(),task);
            task.direction = "receive"; task.name = name; task.count = 1;
            return ui(R.string.ui_saved)+name;
        });
    }
    record UploadFile(Uri uri, ResumableUpload.Metadata metadata) {}
    boolean upload(String url, String host, String pin, String token, List<UploadFile> selected) {
        if (transferring) return false;
        List<UploadFile> files = java.util.Collections.unmodifiableList(new java.util.ArrayList<>(selected));
        if (files.isEmpty() || files.size() > ShareInbox.LIMIT) return false;
        java.util.Set<String> authorities = new java.util.HashSet<>();
        for (UploadFile file : files) authorities.add(ProviderTasks.authority(file.uri().toString()));
        final String usbKey = selectedBase.isEmpty() ? new ComputerStore(this).usbAccess(url) : selectedUsbAccess;
        return transfer(url,ui(R.string.ui_preparing_to_send_files),"",true,authorities,(lease,task) -> {
            PinnedConnections network = new PinnedConnections(host,pin,url,usbKey);
            task.total = files.size(); String lastName = "";
            for (int i=0;i<files.size();i++) {
                final int completed=i; final String position=(i+1)+"/"+files.size()+" · ";
                task.check();
                ResumableUpload.Receipt receipt=ResumableUpload.send(getContentResolver(),network,url,token,files.get(i).uri(),files.get(i).metadata(),task,() -> micActive,
                    (label,value,retrying) -> progress(task,ui(R.string.ui_sending)+position+label+(retrying ? ui(R.string.ui_resuming) : ""),ResumableUpload.batchProgress(completed,files.size(),value)));
                lastName=receipt.name();
            }
            if (task.cancelled.get()) return AppLanguage.quantity(this,R.plurals.ui_upload_cancel_completed,task.completed,task.completed);
            if (files.size()==1 && !lastName.isEmpty()) return ui(R.string.ui_sent)+lastName;
            return ui(R.string.ui_sent)+files.size()+ui(R.string.ui_files);
        });
    }
    void cancelTransfer() {
        TransferTask task=currentTransfer;
        if (!transferring || task==null || !task.cancel()) return;
        transferMessage=ui(R.string.ui_canceling_transfer); emitTransfer(false);
    }
    private void emit(String name, JSONObject state) {
        try {
            if (!state.has("owner")) state.put("owner", "plumelink-native-transfer".equals(name) ? transferOwner : "plumelink-microphone".equals(name) ? baseOwner : selectedOwner);
            if (!state.has("transport")) state.put("transport", "plumelink-native-transfer".equals(name) ? transferTransport : "plumelink-microphone".equals(name) ? baseTransport : selectedTransport);
        }
        catch (org.json.JSONException ignored) {}
        if (listener != null) listener.event(name, state);
    }
    private void emitMic(boolean isSnapshot) {
        emitMic(isSnapshot, baseOwner, baseTransport);
    }
    private void emitMic(boolean isSnapshot, String owner, String transport) {
        try { emit("plumelink-microphone", new JSONObject().put("owner",owner).put("transport",transport).put("id", request).put("phase", phase).put("level", level)
            .put("message", boundedMessage(micMessage)).put("locked", locked).put("began", began).put("snapshot", isSnapshot)); } catch (org.json.JSONException ignored) {}
    }
    private boolean transferSucceeded;
    private void refuseTransfer(String origin) {
        try { emit("plumelink-native-transfer", new JSONObject().put("owner",selectedOwner).put("transport",selectedTransport).put("origin", origin).put("succeeded", false).put("done", true)
            .put("progress", 0).put("message", ui(R.string.ui_previous_pc_tasks_are_stopping_try_again_shortly)).put("snapshot", false)); }
        catch (org.json.JSONException ignored) {}
    }
    private void emitTransfer(boolean isSnapshot) { emitTransfer(isSnapshot, isSnapshot && transferSucceeded); }
    private void emitTransfer(boolean isSnapshot, boolean succeeded) {
        emitTransfer(isSnapshot, succeeded, transferOwner, transferTransport);
    }
    private void emitTransfer(boolean isSnapshot, boolean succeeded, String owner, String transport) {
        if (!isSnapshot) transferSucceeded = succeeded;
        try { emit("plumelink-native-transfer", new JSONObject().put("owner",owner).put("transport",transport).put("origin", transferBase).put("succeeded", succeeded).put("done", !transferring).put("progress", percent)
            .put("message", boundedMessage(transferMessage)).put("snapshot", isSnapshot).put("direction", transferDirection)
            .put("fileName", transferName).put("fileCount", transferCount)); } catch (org.json.JSONException ignored) {}
    }
    private static String boundedMessage(String value) {
        if (value == null || value.isEmpty()) return "";
        return value.length() <= 512 ? value : value.substring(0, 512);
    }
    private PendingIntent action(String name) {
        return PendingIntent.getService(this, name.hashCode(), new Intent(this, RemoteSessionService.class).setAction(name), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }
    private boolean updateForeground() {
        main.removeCallbacks(renew);
        if (destroyed || (!micActive && !transferring && !keepAlive)) {
            retireForeground(); return false;
        }
        int requested = 0;
        if (Build.VERSION.SDK_INT >= 29) {
            if (keepAlive) requested |= ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE;
            if (transferring) requested |= ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC;
            if (micActive && Build.VERSION.SDK_INT >= 30) requested |= ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE;
            if (requested == 0) requested = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE;
        } else requested = 1;
        try {
            notificationAt = SystemClock.elapsedRealtime();
            PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
            String text = micActive ? ("connecting".equals(phase) ? ui(R.string.mic_connecting)
                : "reconnecting".equals(phase) ? ui(R.string.mic_reconnecting)
                : locked ? ui(R.string.ui_continuous_microphone_in_use) : ui(R.string.ui_phone_microphone_in_use))
                : transferring ? boundedMessage(transferMessage) : ui(R.string.ui_remote_connection_active);
            if (micActive && !micMessage.isEmpty()) text = boundedMessage(micMessage);
            if (micActive && transferring) text = boundedMessage(text + ui(R.string.ui_file_transfer) + percent + "%");
            Notification.Builder builder = new Notification.Builder(this, CHANNEL).setSmallIcon(R.drawable.ic_session)
                .setContentTitle(ui(R.string.app_name)).setContentText(text).setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true)
                .setCategory(Notification.CATEGORY_SERVICE).setVisibility(Notification.VISIBILITY_PRIVATE);
            if (micActive) builder.addAction(new Notification.Action.Builder(null, ui(R.string.ui_stop_microphone), action(STOP_MIC)).build());
            if (transferring) { builder.setProgress(100, percent, false); builder.addAction(new Notification.Action.Builder(null, ui(R.string.ui_cancel_transfer), action(CANCEL)).build()); }
            Notification notification = builder.build();
            foregroundTypes = ForegroundPromotion.apply(foregroundTypes, requested,
                types -> promoteForeground(notification, types), () -> notifyForeground(notification));
        } catch (RuntimeException ignored) {
            // Notification construction cannot grant a new service type.
        }
        int connectionType = Build.VERSION.SDK_INT >= 29 ? ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE : 1;
        if (keepAlive && !ForegroundPromotion.covers(foregroundTypes, connectionType)) {
            keepAlive = false;
            keepAliveFailed = true;
            stopBackgroundConnection();
            emitKeepAlive();
            if (!micActive && !transferring) { retireForeground(); return false; }
            requested &= ~connectionType;
        }
        if (foregroundTypes != 0) renew.run();
        return ForegroundPromotion.covers(foregroundTypes, requested);
    }

    private void retireForeground() {
        main.removeCallbacks(renew);
        if (wake.isHeld()) wake.release();
        if (wifiLock != null && wifiLock.isHeld()) wifiLock.release();
        foregroundTypes = 0;
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }
    void promoteForeground(Notification notification, int types) {
        if (Build.VERSION.SDK_INT >= 29) startForeground(NOTIFICATION, notification, types);
        else startForeground(NOTIFICATION, notification);
    }
    void notifyForeground(Notification notification) {
        getSystemService(NotificationManager.class).notify(NOTIFICATION, notification);
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
        for (Thread worker : recoveryThreads.values()) worker.interrupt();
        recoveryThreads.clear(); recoveryTasks.clear();
        if (wake.isHeld()) wake.release();
        if (wifiLock != null && wifiLock.isHeld()) wifiLock.release();
        super.onDestroy();
    }
}
