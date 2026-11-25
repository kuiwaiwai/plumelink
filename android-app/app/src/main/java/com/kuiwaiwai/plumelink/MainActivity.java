package com.kuiwaiwai.plumelink;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.app.AlertDialog;
import android.app.KeyguardManager;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.NetworkRequest;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.DocumentsContract;
import android.view.View;
import android.view.KeyEvent;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.webkit.WebChromeClient;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.widget.FrameLayout;
import android.widget.Toast;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

public final class MainActivity extends Activity implements ConnectScreenUi.Host, PinnedWebViewClient.Host, AndroidTransferBridge.Host {
    private static final int MIN_WEBVIEW_VERSION = 85;
    private static final int PICK_RECEIVE_TREE = 8127;
    private static final int CONFIRM_SENSITIVE = 8128;
    private static final int PICK_UPLOAD_FILES = 8129;
    private static final int REQUEST_MICROPHONE = 8130;
    private static final int SCAN_PAIRING = 8131;
    private static final String STATE_ACTIVE_PC = "active-pc";
    private static final String STATE_USB_ACCESS = "active-usb-access";
    private static final String STATE_UPLOAD = "upload";
    private static final String STATE_SHARED_FILES = "shared-files";
    private static final String STATE_RECEIVE_PICKER = "receive-picker";

    private String ui(int id) { return AppLanguage.text(this, id); }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }

    private AppWindowLayout root;
    private ComputerStore store;
    private LanDiscovery discovery;
    private ConnectScreenUi connectUi;
    private PinnedWebViewClient webClient;

    private volatile WebView webView;
    private final BridgeAccess<WebView> bridgeAccess = new BridgeAccess<>(
        () -> isDestroyed() || isFinishing() ? null : webView, this::runOnUiThread);
    private volatile String expectedHost = "";
    private volatile String currentBaseUrl = "";
    private volatile String currentComputerIdentity = "";
    private volatile String currentUsbAccess = "";
    private String pendingUsbAccess = "", probingUsbAccess = "";
    private String pendingPairUrl = "";
    private String pendingPairCode = "";
    private long pendingPairAt;

    private String pendingSensitiveAction = "";
    private java.lang.ref.WeakReference<WebView> pendingSensitiveSource;
    private boolean keepRemoteAwake;
    private volatile boolean activityResumed;
    private volatile int inputEpoch = 1;
    private int pendingMicrophoneRequest;
    private java.lang.ref.WeakReference<WebView> pendingMicrophoneSource;
    private android.webkit.ValueCallback<Uri[]> uploadPicker;
    private Bundle uploadTarget;
    private boolean receivePickerPending;
    private volatile int receiveDirectoryCheck;
    private volatile String unavailableReceiveTree = "";
    private final ShareInbox shareInbox = new ShareInbox();
    private android.os.CancellationSignal shareReadSignal;
    private java.util.concurrent.atomic.AtomicBoolean shareReadCancelled;
    private String shareReadOwner = "";
    private Runnable shareReadTimeout;
    private int shareNotice;
    private boolean sharedFilesOpenPending;
    private boolean fileTransferAllowed;
    private volatile String sharedFilesSnapshot = "{\"revision\":\"\",\"files\":[],\"ready\":false,\"message\":\"\"}";
    private volatile RemoteSessionService session;
    private boolean sessionBound;
    private NativeRemoteUi nativeRemoteUi;
    private boolean connectionKeepAliveRequested;
    private ConnectivityManager.NetworkCallback networkCallback;
    private boolean networkRegistered;
    private int networkGeneration;
    private int recoveryNetworkRevision, recoveryGeneration;
    private LanRecovery connectionRecovery;
    private LanDiscovery recoveryDiscovery;
    private ConnectionProbe recoveryProbe;
    private Runnable recoveryDeadline;
    private java.util.function.IntFunction<String> connectionRecoveryText = this::ui;
    private final LanRecovery.Draft<WebView> recoveredDraft = new LanRecovery.Draft<>();
    private final java.util.concurrent.ExecutorService recoveryProbes = new java.util.concurrent.ThreadPoolExecutor(
        1, 1, 0, java.util.concurrent.TimeUnit.MILLISECONDS, new java.util.concurrent.ArrayBlockingQueue<>(1),
        task -> new Thread(task, "PlumeLink-connection-recovery"));
    private final LocalNetworks<Network> localNetworks = new LocalNetworks<>();
    private boolean hasShownConnectScreen;
    private int connectionGeneration;
    private String probingUrl = "";
    private String probingIdentity = "";
    private ConnectionProbe connectionProbe;
    private AlertDialog connectionTrustDialog;
    private String connectionStatus = "";
    private Runnable connectionDeadline;

    private final android.content.ServiceConnection sessionConnection = new android.content.ServiceConnection() {
        @Override public void onServiceConnected(android.content.ComponentName name, android.os.IBinder binder) {
            if (isDestroyed() || isFinishing()) return;
            session = ((RemoteSessionService.LocalBinder) binder).service();
            if (!session.attachControlHost(MainActivity.this, activityResumed)) return;
            if (webView != null) session.selectComputer(currentBaseUrl,currentComputerIdentity,store.trustedPin(expectedHost,currentComputerIdentity),currentUsbAccess);
            if (webView != null) session.setControlCredentials(store.token(currentComputerIdentity));
            if (connectionKeepAliveRequested && webView != null) session.setConnectionKeepAlive(true);
            if (!activityResumed) session.background(MainActivity.this);
            String running = session.activeBase();
            if (!running.isEmpty() && webView == null && probingUrl.isEmpty()) connect(running,session.computerIdentity());
            session.listen((event, state) -> {
                if (activityResumed && webView != null && !isDestroyed() && SessionIdentity.accepts(state.optString("owner"),currentSessionOwner()) && SessionIdentity.accepts(state.optString("transport"),SessionIdentity.transport(currentBaseUrl,currentSessionOwner(),currentUsbAccess)) && acceptsSessionEvent(event, state.optString("origin"), currentBaseUrl, session.activeBase())) {
                    webView.evaluateJavascript("window.dispatchEvent(new CustomEvent(" + org.json.JSONObject.quote(event) + ",{detail:" + state + "}))", null);
                }
                if ("plumelink-native-transfer".equals(event) && state.optBoolean("done") && !state.optBoolean("snapshot")) {
                    refreshReceiveDirectory();
                }
            });
            if (activityResumed && webView != null) webView.evaluateJavascript("window.dispatchEvent(new Event('pageshow'))", null);
        }
        @Override public void onServiceDisconnected(android.content.ComponentName name) { session = null; }
    };

    static boolean acceptsSessionEvent(String event, String source, String current, String active) {
        if ("plumelink-download-recovery".equals(event)) return true;
        if ("plumelink-native-transfer".equals(event)) return !source.isEmpty() && source.equals(current);
        return active.isEmpty() || active.equals(current);
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        if (Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
        else getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LAYOUT_STABLE | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        setSystemBars(AppAppearance.dark(this));

        root = new AppWindowLayout(this);
        setContentView(root);
        root.setKeyboardListener(visible -> {
            if (nativeRemoteUi != null) nativeRemoteUi.setKeyboardVisible(visible);
        });
        root.requestApplyInsets();

        store = new ComputerStore(this);
        if (savedInstanceState != null) {
            uploadTarget = savedInstanceState.getBundle(STATE_UPLOAD);
            ArrayList<String> shared = savedInstanceState.getStringArrayList(STATE_SHARED_FILES);
            if (shared != null && shared.size() <= ShareInbox.LIMIT) shareInbox.add(shared);
            receivePickerPending = savedInstanceState.getBoolean(STATE_RECEIVE_PICKER);
        }
        discovery = new LanDiscovery(this, () -> {
            if (connectUi != null) connectUi.renderDiscoveredComputers();
        });
        connectUi = new ConnectScreenUi(this, root, store, discovery, this);
        webClient = new PinnedWebViewClient(this, store, this);

        if (savedInstanceState == null) receiveSharedIntent(getIntent());
        readNextSharedFile();
        emitSharedFiles(false);

        bindSession();
        String incoming = savedInstanceState == null ? connectionFromIntent(getIntent())
            : savedInstanceState.getString(STATE_ACTIVE_PC, "");
        if (!incoming.isEmpty()) {
            if (savedInstanceState != null) { pendingPairUrl=incoming; pendingUsbAccess=savedInstanceState.getString(STATE_USB_ACCESS, ""); }
            connect(incoming);
            return;
        }
        connectUi.enableAutoConnectLast(savedInstanceState == null && !ShareIntents.matches(getIntent()) && shareInbox.empty() && !store.lastUrl().isEmpty());
        showConnectScreen("", "");
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        String endpoint = !probingUrl.isEmpty() ? probingUrl : webView != null ? currentBaseUrl : "";
        if (!endpoint.isEmpty()) outState.putString(STATE_ACTIVE_PC, endpoint);
        outState.putString(STATE_USB_ACCESS,!probingUrl.isEmpty() ? probingUsbAccess : currentUsbAccess);
        if (uploadTarget != null) outState.putBundle(STATE_UPLOAD, new Bundle(uploadTarget));
        if (!shareInbox.empty()) outState.putStringArrayList(STATE_SHARED_FILES, shareInbox.uris());
        outState.putBoolean(STATE_RECEIVE_PICKER, receivePickerPending);
    }

    private void bindSession() {
        if (!sessionBound) sessionBound = bindService(new Intent(this, RemoteSessionService.class), sessionConnection, BIND_AUTO_CREATE);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        if (receiveSharedIntent(intent)) return;
        String incoming = connectionFromIntent(intent);
        if (!incoming.isEmpty()) connect(incoming);
    }

    private String connectionFromIntent(Intent intent) {
        return acceptPairingLink(intent == null || !Intent.ACTION_VIEW.equals(intent.getAction()) || intent.getData() == null ? null : intent.getData().toString());
    }

    private synchronized String acceptPairingLink(String contents) {
        pendingPairUrl = "";
        pendingPairCode = "";
        pendingUsbAccess = "";
        PairingLink link = PairingLink.parse(contents);
        if (link == null) return "";
        String url = EndpointUtils.normalize(link.address);
        if (!EndpointUtils.isValidPrivateEndpoint(url)) return "";
        pendingPairUrl = url;
        pendingPairCode = link.code;
        pendingUsbAccess = link.usbAccess;
        pendingPairAt = android.os.SystemClock.elapsedRealtime();
        return url;
    }

    @Override public void onConnectRequested(String url) { connect(url); }
    @Override public void onConnectRequested(String url, String identity) { connect(url,identity); }
    @Override public void onScanRequested() { scanPairingCode(); }
    @Override public void onLanguageSelected(String value) { setLanguagePreference(value); }
    @Override public void onRefreshScreen(String initial, String errorMessage) { showConnectScreen(initial, errorMessage); }
    @Override public boolean isWebViewActive() { return webView != null; }
    @Override public String connectingUrl() { return probingUrl; }

    @Override public String currentBaseUrl() { return currentBaseUrl; }
    @Override public String expectedHost() { return expectedHost; }
    @Override public String currentComputerIdentity() { return currentComputerIdentity; }
    @Override public boolean isActiveWebView(WebView view) { return view == webView; }
    @Override public void onBindIdentity(String url, String host, String stableId, String pin) { bindIdentity(url, host, stableId, pin); }
    @Override public void onReturnToConnect(String url, String message) { showConnectScreen(url, message); }
    @Override public void onVerifyComputer(String url) {
        String identity = currentComputerIdentity, access = currentUsbAccess;
        showConnectScreen(url, "");
        if (EndpointUtils.isUsbEndpoint(url)) store.setUsbAccess(url, access);
        connect(url, identity);
    }
    @Override public void onRenderProcessCrashed(String recoverUrl) {
        destroyWebView();
        showConnectScreen(recoverUrl, ui(R.string.ui_page_rendering_failed_returned_to_the_connection_screen));
    }
    @Override public AlertDialog showSecurityDialog(AlertDialog.Builder builder) { return connectUi.showSecurityDialog(builder); }
    @Override public void runOnMain(Runnable task) { runOnUiThread(task); }

    @Override public String getLanguagePreference() { return AppLanguage.preference(this); }
    @Override public void setLanguagePreference(String value) {
        AppLanguage.set(this, value);
        runOnUiThread(() -> { if (session != null) session.languageChanged(); emitSharedFiles(false); });
    }
    @Override public String getSystemLanguage() { return getResources().getConfiguration().getLocales().toLanguageTags(); }
    @Override public boolean hasMicrophonePermission() { return checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED; }
    @Override public void scanPairingCode() { runOnUiThread(this::launchScanner); }
    @Override public boolean isAwaitingCertificateTrust() { return webClient != null && webClient.isAwaitingTrust(); }
    @Override public BridgeAccess<WebView> bridgeAccess() { return bridgeAccess; }
    @Override public synchronized String takePairingCode(WebView source) {
        String base = currentBaseUrl;
        if (source == null || source != webView || !base.equals(pendingPairUrl)) return "";
        String code = android.os.SystemClock.elapsedRealtime() - pendingPairAt < 60000 ? pendingPairCode : "";
        pendingPairUrl = "";
        pendingPairCode = "";
        return code;
    }
    @Override public void startMicrophone(int request) {
        runOnUiThread(() -> {
            if (!activityResumed || webView == null || request <= 0) return;
            WebView target = webView;
            if (session == null) { microphoneEvent(target, request, "error", 0, ui(R.string.ui_background_service_is_starting_try_again)); return; }
            if (pendingMicrophoneSource != null || checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
                requestMicrophonePermission(target, request);
                return;
            }
            String token = store.token(currentComputerIdentity);
            if (token.isEmpty()) { microphoneEvent(target, request, "error", 0, ui(R.string.ui_connect_to_a_pc_first)); return; }
            session.startMicrophone(currentBaseUrl, expectedHost, store.trustedPin(expectedHost, currentComputerIdentity), token, request);
        });
    }
    void requestMicrophonePermission(WebView target, int request) {
        if (target == null || target != webView || request <= 0 || isFinishing() || isDestroyed()) return;
        microphoneEvent(target, request, "permission", 0, "");
        if (pendingMicrophoneSource != null) return;
        pendingMicrophoneSource = new java.lang.ref.WeakReference<>(target);
        pendingMicrophoneRequest = request;
        try { requestPermissions(new String[]{android.Manifest.permission.RECORD_AUDIO}, REQUEST_MICROPHONE); }
        catch (RuntimeException error) {
            pendingMicrophoneSource = null; pendingMicrophoneRequest = 0;
            microphoneEvent(target, request, "error", 0, ui(R.string.ui_microphone_permission_denied_enable_it_in_settings));
        }
    }
    WebView takeMicrophonePermissionSource() {
        WebView source = pendingMicrophoneSource == null ? null : pendingMicrophoneSource.get();
        pendingMicrophoneSource = null; pendingMicrophoneRequest = 0;
        return source != null && source == webView && !isFinishing() && !isDestroyed() ? source : null;
    }
    @Override public void stopMicrophone(int request) { runOnUiThread(() -> { if (session != null) session.stopMicrophone(request); }); }
    @Override public void lockMicrophone(int request) {
        runOnUiThread(() -> {
            if (session != null && activityResumed && session.lockMicrophone(request)) requestTaskNotifications();
        });
    }
    @Override public void syncSession() { runOnUiThread(() -> { if (session != null) session.snapshot(); }); }
    @Override public String getComputerName() { return store.computerName(currentComputerIdentity); }
    @Override public void haptic() {
        runOnUiThread(() -> { if (webView != null && activityResumed) webView.performHapticFeedback(android.view.HapticFeedbackConstants.KEYBOARD_TAP); });
    }
    @Override public void setKeepAwake(boolean enabled) {
        runOnUiThread(() -> {
            keepRemoteAwake = enabled && activityResumed && webView != null && !isFinishing();
            if (keepRemoteAwake) getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            else getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        });
    }
    @Override public void setImmersive(boolean enabled) { runOnUiThread(() -> setImmersiveNavigation(enabled)); }
    @Override public boolean isSystemDark() { return AppAppearance.systemDark(this); }
    @Override public String getThemePreference() { return AppAppearance.preference(this); }
    @Override public void setThemePreference(String value) { AppAppearance.save(this, value); }
    @Override public void setAppearance(boolean dark) { runOnUiThread(() -> setSystemBars(dark)); }
    @Override public String getDeviceName() {
        return DeviceNames.read(this, ui(R.string.ui_android_device));
    }
    @Override public String getDeviceModel() { return DeviceNames.clean(Build.MODEL); }
    @Override public String getStoredToken() { return store.token(currentComputerIdentity); }
    @Override public String getUsbAccessKey() { return currentUsbAccess; }
    @Override public int getInputEpoch() { return inputEpoch; }
    @Override public void setStoredToken(String value) {
        store.setToken(currentComputerIdentity, value);
        String identity = currentComputerIdentity;
        runOnUiThread(() -> {
            if (session == null || !identity.equals(currentComputerIdentity) || !value.equals(store.token(identity))) return;
            if (value.isEmpty()) session.stopAll();
            else session.setControlCredentials(value);
        });
    }
    @Override public String getStoredDeviceId() { return store.deviceId(currentComputerIdentity); }
    @Override public void setStoredDeviceId(String value) { store.setDeviceId(currentComputerIdentity, value); }
    @Override public void showPrivacyPolicy() { runOnUiThread(connectUi::showPrivacyPolicy); }
    @Override public String getKeepAlivePreference() { return store.keepAlivePreference(); }
    @Override public void setKeepAlivePreference(WebView source, boolean enabled) { runOnUiThread(() -> {
        if (source == webView) {
            store.setKeepAlivePreference(enabled);
            if (enabled && session != null) session.retryConnectionKeepAlive();
            if (!enabled) {
                connectionKeepAliveRequested = false;
                if (session != null) session.setConnectionKeepAlive(false);
            }
        }
    }); }
    @Override public String getConnectionKeepAliveState() {
        try {
            org.json.JSONObject state = new org.json.JSONObject(session == null ? "{}" : session.connectionKeepAliveState(
                store.token(currentComputerIdentity), currentSessionOwner(), SessionIdentity.transport(currentBaseUrl,currentSessionOwner(),currentUsbAccess)));
            android.os.PowerManager power = getSystemService(android.os.PowerManager.class);
            state.put("batteryRestricted", power != null && !power.isIgnoringBatteryOptimizations(getPackageName()));
            return state.toString();
        } catch (Exception ignored) { return "{}"; }
    }
    @Override public boolean isControlForegroundReady() {
        return activityResumed && session != null && session.isControlHost(this);
    }
    @Override public void openBackgroundSettings() { runOnUiThread(() -> {
        if (!activityResumed || isFinishing()) return;
        try { startActivity(new Intent(android.provider.Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)); }
        catch (RuntimeException unavailable) {
            try { startActivity(new Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:" + getPackageName()))); }
            catch (RuntimeException ignored) { Toast.makeText(this, ui(R.string.ui_could_not_open_background_settings), Toast.LENGTH_LONG).show(); }
        }
    }); }
    @Override public void setConnectionKeepAlive(WebView source, boolean enabled) { runOnUiThread(() -> {
        if (source != webView) return;
        connectionKeepAliveRequested = enabled;
        if (session != null) {
            session.setControlCredentials(store.token(currentComputerIdentity));
            session.setConnectionKeepAlive(connectionKeepAliveRequested);
        }
        if (connectionKeepAliveRequested && activityResumed) requestTaskNotifications();
    }); }
    @Override public String getReceiveFolder() { return receiveFolderLabel(); }
    @Override public String getMobilePreferences() { return store.mobilePreferences(currentComputerIdentity); }
    @Override public void setMobilePreferences(String value) { store.setMobilePreferences(currentComputerIdentity, value); }
    @Override public boolean hasReceiveFolder() {
        String tree = store.receiveTree();
        return !tree.isEmpty() && !tree.equals(unavailableReceiveTree);
    }
    @Override public void refreshReceiveFolder() { refreshReceiveDirectory(); }
    @Override public void chooseReceiveFolder() {
        runOnUiThread(() -> {
            if (receivePickerPending || uploadTarget != null || !activityResumed || webView == null) return;
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            String selected = store.receiveTree();
            if (!selected.isEmpty() && !selected.equals(unavailableReceiveTree)) {
                try {
                    Uri tree = Uri.parse(selected);
                    intent.putExtra(DocumentsContract.EXTRA_INITIAL_URI,
                        DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree)));
                } catch (IllegalArgumentException ignored) {}
            }
            receivePickerPending = true;
            try { startActivityForResult(intent, PICK_RECEIVE_TREE); }
            catch (android.content.ActivityNotFoundException ignored) {
                receivePickerPending = false;
                notifyTransfer(ui(R.string.ui_system_folder_picker_unavailable), receiveFolderLabel());
            }
        });
    }
    @Override public void changeComputer() { runOnUiThread(() -> showConnectScreen(currentBaseUrl, "")); }
    @Override public void startDownload(String path, String fileName, String token) {
        runOnUiThread(() -> {
            String tree = store.receiveTree();
            if (tree.isEmpty() || tree.equals(unavailableReceiveTree)) {
                notifyTransfer(ui(R.string.ui_choose_an_android_download_folder_first), receiveFolderLabel(), true);
                return;
            }
            if (session == null || !activityResumed) {
                notifyTransfer(ui(R.string.ui_return_to_the_app_and_retry), receiveFolderLabel(), true);
                return;
            }
            session.download(currentBaseUrl, expectedHost, store.trustedPin(expectedHost, currentComputerIdentity), path, fileName, token, tree);
            requestTaskNotifications();
        });
    }
    @Override public void cancelFileTransfer() { runOnUiThread(() -> { if (session != null) session.cancelTransfer(); }); }
    @Override public void resolveUnfinishedDownload(String id, String version, boolean keep, String request) {
        if (session != null && webView != null) session.resolveDownload(id, version, keep, request);
    }
    @Override public void confirmSensitiveAction(String action) {
        runOnUiThread(() -> {
            if (!pendingSensitiveAction.isEmpty()) {
                if (pendingSensitiveSource == null || pendingSensitiveSource.get() != webView) notifySensitiveAction(action, false);
                return;
            }
            KeyguardManager keyguard = (KeyguardManager) getSystemService(KEYGUARD_SERVICE);
            Intent intent = keyguard == null ? null : keyguard.createConfirmDeviceCredentialIntent(ui(R.string.ui_confirm_pc_shutdown), ui(R.string.ui_verify_your_identity_to_send_the_shutdown_request));
            if (intent == null) { notifySensitiveAction(action, true); return; }
            pendingSensitiveAction = action;
            pendingSensitiveSource = new java.lang.ref.WeakReference<>(webView);
            try { startActivityForResult(intent, CONFIRM_SENSITIVE); }
            catch (android.content.ActivityNotFoundException ignored) {
                pendingSensitiveAction = "";
                pendingSensitiveSource = null;
                notifySensitiveAction(action, true);
            }
        });
    }
    @Override public void notifyInvalidDownload() { notifyTransfer(ui(R.string.ui_invalid_download_request), receiveFolderLabel(), true); }
    @Override public void notifyInvalidFileName() { notifyTransfer(ui(R.string.ui_invalid_file_name), receiveFolderLabel(), true); }

    private void launchScanner() {
        if (!getPackageManager().hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)) {
            Toast.makeText(this, ui(R.string.ui_no_camera_available_connect_by_address), Toast.LENGTH_LONG).show();
            return;
        }
        try {
            com.journeyapps.barcodescanner.ScanOptions options = new com.journeyapps.barcodescanner.ScanOptions()
                .setCaptureActivity(PairScanActivity.class)
                .setDesiredBarcodeFormats(com.journeyapps.barcodescanner.ScanOptions.QR_CODE)
                .setOrientationLocked(false)
                .setBeepEnabled(false);
            startActivityForResult(options.createScanIntent(this), SCAN_PAIRING);
        } catch (RuntimeException error) {
            Toast.makeText(this, ui(R.string.ui_could_not_open_the_camera_connect_by_address), Toast.LENGTH_LONG).show();
        }
    }

    private void setImmersiveNavigation(boolean enabled) {
        if (Build.VERSION.SDK_INT >= 30) {
            android.view.WindowInsetsController bars = getWindow().getInsetsController();
            if (bars != null) {
                bars.setSystemBarsBehavior(android.view.WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                if (enabled) bars.hide(android.view.WindowInsets.Type.navigationBars());
                else bars.show(android.view.WindowInsets.Type.navigationBars());
            }
        } else {
            View decor = getWindow().getDecorView();
            int flags = decor.getSystemUiVisibility();
            if (enabled) flags |= View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY;
            else flags &= ~(View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY);
            decor.setSystemUiVisibility(flags);
        }
    }

    private void setSystemBars(boolean dark) {
        int flags = getWindow().getDecorView().getSystemUiVisibility();
        flags &= ~(View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR);
        if (!dark) flags |= View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
        getWindow().getDecorView().setSystemUiVisibility(flags);
        getWindow().setStatusBarColor(UiStyle.background(dark));
        getWindow().setNavigationBarColor(dark ? Color.rgb(24, 28, 36) : Color.rgb(240, 242, 248));
        if (root != null) root.setBackgroundColor(UiStyle.background(dark));
    }

    private void destroyWebView() {
        destroyWebView(true);
    }
    private void destroyWebView(boolean stopKeepAlive) {
        cancelCurrentConnectionRecovery();
        recoveredDraft.clear();
        fileTransferAllowed = false;
        WebView previous = webView;
        webView = null;
        clearConnectionDeadline();
        connectionStatus = "";
        if (nativeRemoteUi != null) {
            nativeRemoteUi.destroy();
            nativeRemoteUi = null;
        }
        if (previous == null) return;
        if (stopKeepAlive) {
            connectionKeepAliveRequested = false;
            if (session != null) session.setConnectionKeepAlive(false);
        }
        if (uploadPicker != null) {
            uploadPicker.onReceiveValue(null);
            uploadPicker = null;
        }
        if (previous.getParent() instanceof ViewGroup) ((ViewGroup) previous.getParent()).removeView(previous);
        previous.stopLoading();
        previous.destroy();
    }

    private void showConnectScreen(String initial, String errorMessage) {
        abandonUpload();
        connectionGeneration++;
        cancelConnectionProbe();
        probingUrl = "";
        synchronized (this) {
            pendingPairUrl = "";
            pendingPairCode = "";
        }
        if (webClient != null) webClient.clearPrompt();
        if (session != null && webView != null) session.stopAll();
        discovery.stop();
        discovery.resetState();
        hasShownConnectScreen = true;
        setImmersiveNavigation(false);
        setSystemBars(AppAppearance.dark(this));
        keepRemoteAwake = false;
        getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        destroyWebView();
        connectUi.show(initial, errorMessage);
        discovery.start();
    }

    private void bindIdentity(String url, String host, String stableId, String pin) {
        String resolved = store.bindTrustedIdentity(url, host, stableId, pin);
        if (resolved != null) currentComputerIdentity = resolved;
    }

    private String appVersionName() {
        try {
            String name = getPackageManager().getPackageInfo(getPackageName(), 0).versionName;
            return name == null || name.isEmpty() ? "0.1.1" : name;
        } catch (Exception ignored) {
            return "0.1.1";
        }
    }

    private String currentSessionOwner() { return SessionIdentity.owner(currentBaseUrl,currentComputerIdentity,store.trustedPin(expectedHost,currentComputerIdentity)); }
    private void connect(String url) { connect(url, ""); }
    private void connect(String url, String requestedIdentity) {
        cancelCurrentConnectionRecovery();
        recoveredDraft.clear();
        String usbAccess = EndpointUtils.isUsbEndpoint(url) ? url.equals(pendingPairUrl) && UsbAccess.validKey(pendingUsbAccess) ? pendingUsbAccess : store.usbAccess(url) : "";
        if (url.equals(probingUrl) && usbAccess.equals(probingUsbAccess) && (requestedIdentity.isEmpty() || requestedIdentity.equals(probingIdentity))) return;
        if (uploadTarget != null && !url.equals(uploadTarget.getString("endpoint"))) abandonUpload();
        connectUi.enableAutoConnectLast(false);
        if (webView != null) {
            if (session != null) session.stopAll();
            destroyWebView();
            hasShownConnectScreen = true;
            setImmersiveNavigation(false);
            connectUi.show(url, "");
        }
        int generation = ++connectionGeneration;
        cancelConnectionProbe();
        if (!EndpointUtils.isValidPrivateEndpoint(url)) { showConnectScreen(url, ui(R.string.ui_pc_unreachable_check_the_service_and_network)); return; }
        probingUrl = url;
        probingIdentity = requestedIdentity;
        probingUsbAccess = usbAccess;
        connectUi.renderConnecting();
        String host = Uri.parse(url).getHost();
        String discovered = EndpointUtils.isUsbEndpoint(url) ? "" : discovery.identityFor(url);
        String identity = requestedIdentity.matches("[a-f0-9]{24}") ? requestedIdentity : discovered.matches("[a-f0-9]{24}") ? discovered : EndpointUtils.isUsbEndpoint(url) ? "" : store.storedIdentityOrEmpty(url);
        String savedPin = store.trustedPin(host, identity);
        ConnectionProbe probe = new ConnectionProbe();
        connectionProbe = probe;
        new Thread(() -> {
            ConnectionProbe.Result result = probe.inspect(url,usbAccess);
            runOnUiThread(() -> {
                if (connectionGeneration != generation || connectionProbe != probe || isFinishing() || isDestroyed()) return;
                connectionProbe = null;
                if (result.state() != ConnectionProbe.State.READY) {
                    showConnectScreen(url, ui(result.state() == ConnectionProbe.State.SECURE_FAILED ? R.string.ui_secure_connection_failed_verify_the_pc_and_its_certific
                        : result.state() == ConnectionProbe.State.USB_EXPIRED ? R.string.ui_usb_connection_expired : result.state() == ConnectionProbe.State.INVALID_IDENTITY ? R.string.ui_not_plumelink_service : R.string.ui_pc_unreachable_check_the_service_and_network));
                    return;
                }
                String actualIdentity = !result.identity().isEmpty() ? result.identity() : !identity.isEmpty() ? identity : store.trustedIdentity(result.pin());
                String currentPin = store.trustedPin(host, actualIdentity);
                ConnectionIdentity.Decision decision = ConnectionIdentity.decide(identity, result.identity(), result.pin(), savedPin, currentPin, !store.trustedIdentity(result.pin()).isEmpty());
                boolean changed = decision == ConnectionIdentity.Decision.IDENTITY_CHANGED || decision == ConnectionIdentity.Decision.CERTIFICATE_CHANGED;
                if (result.pin().isEmpty()) { showConnectScreen(url, ui(R.string.ui_certificate_changed_the_new_certificate_was_not_trusted)); return; }
                if (decision == ConnectionIdentity.Decision.ACCEPT) {
                    acceptConnection(generation, url, host, actualIdentity, result.pin(),usbAccess); return;
                }
                connectionTrustDialog = showSecurityDialog(AppAppearance.dialog(this)
                    .setTitle(ui(changed ? R.string.ui_certificate_changed : R.string.ui_trust_this_pc))
                    .setMessage((changed ? ui(R.string.ui_reinstalled_pc_trust) + "\n\n" : "") + ui(EndpointUtils.isUsbEndpoint(url) ? R.string.ui_verify_usb_pc : R.string.ui_verify_that_this_address_matches_your_pc) + url.replace("https://", "") + ui(R.string.ui_sha_256_certificate_fingerprint) + result.pin())
                    .setNegativeButton(ui(R.string.ui_cancel), (dialog, which) -> rejectConnection(generation, url))
                    .setPositiveButton(ui(R.string.ui_trust_and_connect), (dialog, which) -> {
                        if (connectionGeneration != generation || !url.equals(probingUrl) || isFinishing() || isDestroyed()) return;
                        if (changed && (actualIdentity.equals(identity) || !result.pin().equals(currentPin)))
                            store.prepareTrustReplacement(url, identity, actualIdentity);
                        acceptConnection(generation, url, host, actualIdentity, result.pin(),usbAccess);
                    })
                    .setOnCancelListener(dialog -> rejectConnection(generation, url)));
            });
        }, "PlumeLink-connection-check").start();
    }

    private void cancelConnectionProbe() {
        ConnectionProbe previous = connectionProbe; connectionProbe = null;
        if (previous != null) previous.cancel();
        AlertDialog dialog = connectionTrustDialog; connectionTrustDialog = null;
        if (dialog != null) dialog.dismiss();
    }
    private void rejectConnection(int generation, String url) {
        if (connectionGeneration == generation && url.equals(probingUrl)) showConnectScreen(url, ui(R.string.ui_trust_canceled_choose_another_pc));
    }
    private void acceptConnection(int generation, String url, String host, String identity, String pin, String usbAccess) {
        if (connectionGeneration != generation || !url.equals(probingUrl) || isFinishing() || isDestroyed()) return;
        connectionTrustDialog = null;
        probingUrl = "";
        bindIdentity(url, host, identity, pin);
        if (EndpointUtils.isUsbEndpoint(url)) store.setUsbAccess(url,usbAccess);
        openSession(url, identity);
    }

    private void clearConnectionDeadline() {
        if (connectionDeadline == null) return;
        root.removeCallbacks(connectionDeadline);
        connectionDeadline = null;
    }

    private void startConnectionDeadline(WebView source) {
        connectionDeadline = new Runnable() {
            @Override public void run() {
                if (source != webView || isFinishing() || isDestroyed()) return;
                if (webClient.isAwaitingTrust()) {
                    root.postDelayed(this, 8000);
                    return;
                }
                showConnectScreen(currentBaseUrl, ui(R.string.ui_pc_connection_timed_out_check_the_network_and_retry));
            }
        };
        root.postDelayed(connectionDeadline, 8000);
    }

    private void openSession(String url, String verifiedIdentity) { openSession(url, verifiedIdentity, null); }
    @SuppressLint({"SetJavaScriptEnabled", "SetTextI18n"})
    private void openSession(String url, String verifiedIdentity, String draft) {
        synchronized (this) {
            if (!url.equals(pendingPairUrl)) {
                pendingPairUrl = "";
                pendingPairCode = "";
            }
        }
        if (webClient != null) webClient.clearPrompt();
        android.content.pm.PackageInfo provider = WebView.getCurrentWebViewPackage();
        int webViewVersion = 0;
        try {
            if (provider != null) webViewVersion = Integer.parseInt(provider.versionName.split("\\.")[0]);
        } catch (Exception ignored) {}
        if (webViewVersion < MIN_WEBVIEW_VERSION) {
            showConnectScreen(url, String.format(Locale.ROOT, ui(R.string.ui_update_webview), MIN_WEBVIEW_VERSION));
            return;
        }
        hasShownConnectScreen = false;
        connectUi.enableAutoConnectLast(false);
        connectUi.hide();
        discovery.stop();
        if (session != null && !session.activeBase().isEmpty() && !session.activeBase().equals(url)) session.stopAll();
        destroyWebView();
        Uri uri = Uri.parse(url);
        expectedHost = uri.getHost() == null ? "" : uri.getHost();
        currentBaseUrl = url;
        currentUsbAccess = store.usbAccess(url);
        String storedId = store.storedIdentityOrEmpty(url);
        currentComputerIdentity = verifiedIdentity.matches("^[a-f0-9]{24}$") ? verifiedIdentity
            : (!storedId.isEmpty() ? storedId : url);
        if (session != null) session.selectComputer(url,currentComputerIdentity,store.trustedPin(expectedHost,currentComputerIdentity),currentUsbAccess);
        root.removeAllViews();

        webView = new WebView(this);
        if (draft != null) recoveredDraft.set(webView, currentComputerIdentity, currentSessionOwner(), draft);
        webView.setBackgroundColor(UiStyle.background(AppAppearance.dark(this)));
        WebSettings settings = webView.getSettings();
        settings.setTextZoom(Math.round(getResources().getConfiguration().fontScale * 100));
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        settings.setUserAgentString(settings.getUserAgentString() + " PlumeLink-Android/" + appVersionName());
        WebView.setWebContentsDebuggingEnabled(
            (getApplicationInfo().flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0
        );
        webView.setWebChromeClient(new WebChromeClient() {
            @Override
            public boolean onShowFileChooser(WebView source, android.webkit.ValueCallback<Uri[]> callback, FileChooserParams params) {
                if (source != webView || receivePickerPending || uploadTarget != null || getStoredToken().isEmpty()) {
                    callback.onReceiveValue(null);
                    return true;
                }
                uploadPicker = callback;
                launchUploadPicker(params.getMode() == FileChooserParams.MODE_OPEN_MULTIPLE);
                return true;
            }
        });
        webView.setWebViewClient(webClient);
        webView.addJavascriptInterface(new AndroidTransferBridge(this, webView), "PlumeLinkAndroid");
        webView.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
        webView.setAlpha(0f);
        webView.setFocusable(false);
        root.addView(webView, new FrameLayout.LayoutParams(1, 1));

        webView.loadUrl(url);
        startConnectionDeadline(webView);
    }

    @Override public void chooseUploadFiles() {
        runOnUiThread(() -> {
            if (uploadTarget != null || receivePickerPending || !activityResumed || webView == null || getStoredToken().isEmpty()) return;
            launchUploadPicker(true);
        });
    }

    private void launchUploadPicker(boolean multiple) {
        uploadTarget = new Bundle();
        uploadTarget.putString("endpoint", currentBaseUrl);
        uploadTarget.putString("identity", currentComputerIdentity);
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        try {
            startActivityForResult(intent, PICK_UPLOAD_FILES);
        } catch (android.content.ActivityNotFoundException error) {
            if (uploadPicker != null) uploadPicker.onReceiveValue(null);
            uploadPicker = null;
            uploadTarget = null;
            Toast.makeText(this, ui(R.string.ui_system_file_picker_unavailable), Toast.LENGTH_SHORT).show();
        }
    }

    @Override public void renderNativeUi(WebView source, String json) {
        runOnUiThread(() -> {
            if (source == null || webView != source || isDestroyed() || isFinishing()) return;
            try {
                org.json.JSONObject state = new org.json.JSONObject(json);
                String status = state.optString("sessionStatus");
                if ("connected".equals(status) || "unpaired".equals(status)) clearConnectionDeadline();
                if ("connected".equals(status) && !"connected".equals(connectionStatus)) store.rememberComputer(currentBaseUrl, currentComputerIdentity);
                connectionStatus = status;
                if ("connected".equals(status) || "unpaired".equals(status)) cancelCurrentConnectionRecovery();
                fileTransferAllowed = "connected".equals(status) && state.optBoolean("fileTransferAllowed");
                if (nativeRemoteUi == null) nativeRemoteUi = new NativeRemoteUi(this, root, new NativeRemoteUi.Host() {
                    @Override public void action(String id, Object value) {
                        if (source != webView || (!activityResumed && !passiveEdit(id,value))) return;
                        int marker = Math.max(id.lastIndexOf(":sheet:"), id.lastIndexOf(":screen:"));
                        String logical = marker < 0 ? id : id.substring(0, marker);
                        if ("change".equals(logical) || "pair-change".equals(logical) || "offline-change".equals(logical)) {
                            showConnectScreen(currentBaseUrl, "");
                            return;
                        }
                        org.json.JSONObject event = new org.json.JSONObject();
                        try { event.put("id", id).put("value", value == null ? org.json.JSONObject.NULL : value); }
                        catch (org.json.JSONException ignored) { return; }
                        dispatchNativeUi(source, "plumelink-native-ui", event);
                    }
                    @Override public void pointer(org.json.JSONObject sample) {
                        dispatchNativeUi(source, "plumelink-native-pointer", sample);
                    }
                });
                nativeRemoteUi.setKeyboardVisible(root.keyboardVisible());
                nativeRemoteUi.render(state);
            } catch (org.json.JSONException ignored) {}
        });
    }

    private boolean dispatchNativeUi(WebView source, String type, org.json.JSONObject event) {
        if (source != webView || (!activityResumed && !passiveEdit(event.optString("id"),event.opt("value")))) return false;
        try { event.put("inputEpoch",inputEpoch); } catch (org.json.JSONException ignored) { return false; }
        source.evaluateJavascript("window.dispatchEvent(new CustomEvent(" + org.json.JSONObject.quote(type)
            + ",{detail:" + event + "}))", null);
        return true;
    }
    private static boolean passiveEdit(String id,Object value) {
        String logical=id.split(":(?:screen|sheet):",2)[0];
        return ("draft".equals(logical)||"code".equals(logical)||"file-filter".equals(logical)) && (value instanceof String || value instanceof org.json.JSONObject edit && edit.opt("text") instanceof String && edit.opt("revision") instanceof Number);
    }

    private boolean recoveryBusy() { return uploadTarget != null || session != null && session.hasActiveWork(); }
    private void connectionRecoveryEvent(WebView source, String request, String phase, int message) {
        org.json.JSONObject event = new org.json.JSONObject();
        try { event.put("request", request).put("phase", phase).put("message", connectionRecoveryText.apply(message)); }
        catch (org.json.JSONException ignored) { return; }
        if (source != null) dispatchNativeUi(source, "plumelink-connection-recovery", event);
    }
    private void cancelCurrentConnectionRecovery() {
        LanRecovery previous = connectionRecovery;
        if (previous == null) return;
        connectionRecovery = null; recoveryGeneration++;
        previous.cancel();
    }
    @Override public void cancelConnectionRecovery(WebView source, String request) {
        if (source == webView && connectionRecovery != null && request.equals(connectionRecovery.request())) cancelCurrentConnectionRecovery();
    }
    @Override public String takeRecoveredDraft(WebView source) {
        if (source != webView || store == null) return "";
        return recoveredDraft.take(source, currentComputerIdentity, currentSessionOwner());
    }
    @Override public void recoverConnection(WebView source, String draft, String request) {
        if (source != webView || !activityResumed || isFinishing() || isDestroyed() || !LanRecovery.validDraft(draft)
            || request == null || !request.matches("[a-f0-9]{32}")) return;
        if (!probingUrl.isEmpty() || connectionProbe != null || connectionTrustDialog != null) {
            connectionRecoveryEvent(source, request, "error", R.string.ui_connection_recovery_unavailable); return;
        }
        if (connectionRecovery != null && request.equals(connectionRecovery.request())) return;
        cancelCurrentConnectionRecovery();
        if (recoveryBusy()) { connectionRecoveryEvent(source, request, "error", R.string.ui_connection_recovery_busy); return; }
        String identity = currentComputerIdentity, original = currentBaseUrl;
        String pin = store.trustedPin(expectedHost, identity);
        String token = getStoredToken();
        if (EndpointUtils.isUsbEndpoint(original) || !identity.matches("[a-f0-9]{24}") || pin.isEmpty()
            || !token.matches("[a-f0-9]{64}") || !("offline".equals(connectionStatus) || "connecting".equals(connectionStatus))) {
            connectionRecoveryEvent(source, request, "error", R.string.ui_connection_recovery_unavailable); return;
        }
        int generation = ++recoveryGeneration, connection = connectionGeneration;
        int network = networkGeneration, revision = recoveryNetworkRevision;
        connectionRecovery = new LanRecovery(new LanRecovery.Host() {
            @Override public boolean current() {
                return generation == recoveryGeneration && source == webView && activityResumed && !isDestroyed() && !isFinishing()
                    && connection == connectionGeneration && network == networkGeneration && revision == recoveryNetworkRevision
                    && identity.equals(currentComputerIdentity) && original.equals(currentBaseUrl)
                    && token.equals(getStoredToken()) && pin.equals(store.trustedPin(expectedHost, identity))
                    && ("offline".equals(connectionStatus) || "connecting".equals(connectionStatus));
            }
            @Override public void discover(Runnable update) {
                recoveryDiscovery = new LanDiscovery(MainActivity.this, update);
                recoveryDiscovery.start();
            }
            @Override public java.util.Map<String,String> discovered() {
                java.util.Map<String,String> result = new java.util.LinkedHashMap<>();
                if (recoveryDiscovery != null) for (String url : recoveryDiscovery.computers().keySet()) {
                    if (!EndpointUtils.isUsbEndpoint(url) && EndpointUtils.isValidPrivateEndpoint(url)) result.put(url, recoveryDiscovery.identityFor(url));
                }
                return result;
            }
            @Override public void probe(String url, java.util.function.Consumer<LanRecovery.Result> complete) {
                ConnectionProbe probe = new ConnectionProbe(); recoveryProbe = probe;
                try {
                    recoveryProbes.execute(() -> {
                        ConnectionProbe.Result result = probe.inspect(url);
                        runOnUiThread(() -> {
                            if (recoveryProbe != probe) return;
                            recoveryProbe = null;
                            complete.accept(new LanRecovery.Result(result.state() == ConnectionProbe.State.READY, result.identity(), result.pin()));
                        });
                    });
                } catch (java.util.concurrent.RejectedExecutionException error) {
                    recoveryProbe = null; probe.cancel(); complete.accept(new LanRecovery.Result(false, "", ""));
                }
            }
            @Override public void stopDiscovery() {
                LanDiscovery previous = recoveryDiscovery; recoveryDiscovery = null;
                if (previous != null) try { previous.stop(); } catch (RuntimeException ignored) {}
            }
            @Override public void cancelProbe() { if (recoveryProbe != null) { recoveryProbe.cancel(); recoveryProbe = null; } }
            @Override public void deadline(Runnable timeout) { recoveryDeadline = timeout; root.postDelayed(timeout, 12_000); }
            @Override public void clearDeadline() { if (recoveryDeadline != null) { root.removeCallbacks(recoveryDeadline); recoveryDeadline = null; } }
            @Override public void phase(String phase) {
                connectionRecoveryEvent(source, request, phase, "searching".equals(phase) ? R.string.ui_connection_recovery_searching
                    : "cancelled".equals(phase) ? R.string.ui_connection_recovery_cancelled : R.string.ui_connection_recovery_not_found);
            }
            @Override public void recovered(String url, String value) {
                if (!current()) return;
                connectionRecovery = null;
                if (recoveryBusy()) { connectionRecoveryEvent(source, request, "error", R.string.ui_connection_recovery_busy); return; }
                String host = Uri.parse(url).getHost();
                bindIdentity(url, host, identity, pin);
                openSession(url, identity, value);
            }
        }, identity, pin, original, draft, request);
        connectionRecovery.start();
    }

    private void updateEdgeGestureExclusion(View view) {
        if (Build.VERSION.SDK_INT < 29 || view == null) return;
        int w = view.getWidth();
        int h = view.getHeight();
        if (w <= 0 || h <= 0) return;
        int edge = Math.min(dp(24), w / 6);
        int top = Math.max(0, h / 4);
        int maxBand = dp(190);
        int bottom = Math.min(h, top + Math.min(maxBand, Math.max(dp(80), h / 2)));
        view.setSystemGestureExclusionRects(java.util.Arrays.asList(
            new android.graphics.Rect(0, top, edge, bottom),
            new android.graphics.Rect(w - edge, top, w, bottom)
        ));
    }

    private void notifyTransfer(String message, String folder) {
        notifyTransfer(message, folder, false);
    }

    private void notifyTransfer(String message, String folder, boolean done) {
        WebView target = webView;
        new Handler(Looper.getMainLooper()).post(() -> {
            if (webView == null || webView != target || isDestroyed()) return;
            if (!message.isEmpty()) Toast.makeText(MainActivity.this, message, Toast.LENGTH_SHORT).show();
            String detail = "{message:" + org.json.JSONObject.quote(message) + ",folder:" + org.json.JSONObject.quote(folder) + (done ? ",done:true" : "") + "}";
            webView.evaluateJavascript("window.dispatchEvent(new CustomEvent('plumelink-native-transfer',{detail:" + detail + "}))", null);
        });
    }

    private String receiveFolderLabel() {
        String value = store.receiveTree();
        if (value.isEmpty()) return ui(R.string.ui_choose_a_download_folder);
        if (value.equals(unavailableReceiveTree)) return ui(R.string.ui_android_download_folder_unavailable);
        try {
            Uri tree = Uri.parse(value);
            String path = ReceiveDirectory.labelPath(tree.getAuthority(), DocumentsContract.getTreeDocumentId(tree),
                ui(R.string.ui_internal_storage), ui(R.string.ui_external_storage));
            return ui(R.string.ui_selected) + path;
        } catch (Exception ignored) {
            return ui(R.string.ui_android_download_folder_selected);
        }
    }

    private void refreshReceiveDirectory() {
        String tree = store.receiveTree();
        int check = ++receiveDirectoryCheck;
        if (tree.isEmpty() || isDestroyed()) return;
        ProviderTasks.Lease lease = ProviderTasks.SHARED.tryAcquire(java.util.Collections.singleton(ProviderTasks.authority(tree)));
        if (lease == null) return;
        Runnable timeout = () -> { if (check == receiveDirectoryCheck) receiveDirectoryCheck++; };
        root.postDelayed(timeout, 10_000);
        try {
            new Thread(() -> {
                try (lease) {
                    if (check != receiveDirectoryCheck) return;
                    String unavailable = ReceiveDirectory.writableDocument(getContentResolver(), tree) == null ? tree : "";
                    if (check != receiveDirectoryCheck) return;
                    ReceiveDirectory.releaseUnused(getContentResolver(), store::receiveTree, new PendingDownloads(getFilesDir()));
                    runOnUiThread(() -> {
                        root.removeCallbacks(timeout);
                        if (isDestroyed() || check != receiveDirectoryCheck || !tree.equals(store.receiveTree())
                                || unavailable.equals(unavailableReceiveTree)) return;
                        unavailableReceiveTree = unavailable;
                        if (webView != null) webView.evaluateJavascript("window.dispatchEvent(new Event('plumelink-native-folder'))", null);
                    });
                } finally { runOnUiThread(() -> root.removeCallbacks(timeout)); }
            }, "PlumeLink-folder-check").start();
        } catch (RuntimeException error) { lease.close(); root.removeCallbacks(timeout); }
    }

    private void microphoneEvent(WebView target, int request, String phase, double level, String message) {
        if (target == null || webView != target || isDestroyed()) return;
        String detail = "{id:" + request + ",phase:" + org.json.JSONObject.quote(phase) + ",level:" + level + ",message:" + org.json.JSONObject.quote(message) + "}";
        target.evaluateJavascript("window.dispatchEvent(new CustomEvent('plumelink-microphone',{detail:" + detail + "}))", null);
    }

    private void requestTaskNotifications() {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            if (!store.notificationRequested()) {
                store.markNotificationRequested();
                requestPermissions(new String[]{android.Manifest.permission.POST_NOTIFICATIONS}, 8131);
            } else if (!store.notificationDeniedPrompted()) {
                store.markNotificationDeniedPrompted();
                Toast.makeText(this, ui(R.string.ui_notifications_are_off_enable_them_in_connection_help_fo), Toast.LENGTH_LONG).show();
            }
        }
    }

    @Override
    public void onRequestPermissionsResult(int code, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(code, permissions, results);
        if (code == REQUEST_MICROPHONE) {
            int request = pendingMicrophoneRequest;
            WebView target = takeMicrophonePermissionSource();
            if (target == null) return;
            boolean granted = results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED;
            boolean retry = shouldShowRequestPermissionRationale(android.Manifest.permission.RECORD_AUDIO);
            boolean shown = nativeRemoteUi != null && nativeRemoteUi.showMicrophonePermissionHint(granted, retry, () -> {
                if (target != webView || isDestroyed() || isFinishing()) return;
                if (retry) requestMicrophonePermission(target, request);
                else startActivity(new Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                    android.net.Uri.fromParts("package", getPackageName(), null)));
            });
            microphoneEvent(target, request, "idle", 0, shown ? "" : ui(granted
                ? R.string.ui_microphone_permission_granted_press_and_hold_again : R.string.ui_microphone_permission_denied_enable_it_in_settings));
        }
    }

    private void abandonUpload() {
        if (uploadTarget == null) return;
        // Reserve the request code until the outstanding picker returns.
        uploadTarget = new Bundle();
    }

    private boolean receiveSharedIntent(Intent intent) {
        if (!ShareIntents.matches(intent)) return false;
        try { stageSharedFiles(ShareIntents.files(intent, true)); }
        catch (RuntimeException error) { shareNotice = R.string.ui_share_invalid; emitSharedFiles(true); }
        return true;
    }
    private void stageSharedFiles(List<String> files) {
        shareNotice = shareInbox.add(files) ? 0 : R.string.ui_select_up_to_16_files_at_a_time;
        readNextSharedFile();
        emitSharedFiles(true);
    }
    private void cancelSharedRead() {
        java.util.concurrent.atomic.AtomicBoolean cancelled = shareReadCancelled;
        if (cancelled != null) cancelled.set(true);
        shareReadCancelled = null;
        if (shareReadTimeout != null) root.removeCallbacks(shareReadTimeout);
        shareReadTimeout = null;
        android.os.CancellationSignal signal = shareReadSignal;
        shareReadSignal = null; shareReadOwner = "";
        if (signal != null) ProviderTasks.cancel(signal::cancel);
    }
    private void readNextSharedFile() {
        if (!shareReadOwner.isEmpty() || isDestroyed()) return;
        ShareInbox.File file = shareInbox.files().stream().filter(ShareInbox.File::pending).findFirst().orElse(null);
        if (file == null) return;
        ProviderTasks.Lease lease = ProviderTasks.SHARED.tryAcquire(java.util.Collections.singleton(ProviderTasks.authority(file.uri())));
        if (lease == null) {
            shareInbox.metadata(file, "", -1, R.string.ui_file_provider_busy);
            emitSharedFiles(false); readNextSharedFile(); return;
        }
        readSharedFile(file, lease);
    }
    private void readSharedFile(ShareInbox.File file, ProviderTasks.Lease lease) {
        shareReadOwner = file.id();
        android.os.CancellationSignal signal = new android.os.CancellationSignal();
        java.util.concurrent.atomic.AtomicBoolean cancelled = new java.util.concurrent.atomic.AtomicBoolean();
        shareReadSignal = signal; shareReadCancelled = cancelled;
        Runnable timeout = () -> {
            if (!file.id().equals(shareReadOwner) || isDestroyed()) return;
            shareInbox.metadata(file, "", -1, R.string.ui_share_unreadable);
            shareNotice = R.string.ui_share_unreadable;
            cancelSharedRead(); emitSharedFiles(false); readNextSharedFile();
        };
        shareReadTimeout = timeout;
        root.postDelayed(timeout, 10_000);
        try {
            new Thread(() -> {
                String name = ""; long size = -1; int failure = 0;
                try (lease) {
                    Uri uri = Uri.parse(file.uri());
                    ResumableUpload.Metadata value = ResumableUpload.readMetadata(getContentResolver(), uri, signal, cancelled::get);
                    name = value.name() == null ? "" : value.name(); size = value.size();
                    ResumableUpload.validateMetadata(value);
                    ResumableUpload.checkMetadataCancellation(signal, cancelled::get);
                    try (android.content.res.AssetFileDescriptor source = getContentResolver().openAssetFileDescriptor(uri, "r", signal)) {
                        if (source == null) failure = R.string.ui_share_unreadable;
                    }
                } catch (TransferFailure error) { failure = error.reason(); }
                catch (Exception error) { failure = R.string.ui_share_unreadable; }
                String label = name.replaceAll("[\\p{Cntrl}]", " ");
                if (label.length() > 512) label = label.substring(0, 512);
                final String finalName = label;
                final long finalSize = size;
                final int finalFailure = failure;
                runOnUiThread(() -> {
                    root.removeCallbacks(timeout);
                    if (isDestroyed() || !file.id().equals(shareReadOwner)) return;
                    shareReadOwner = ""; shareReadSignal = null; shareReadCancelled = null; shareReadTimeout = null;
                    shareInbox.metadata(file, finalName, finalSize, finalFailure);
                    emitSharedFiles(false); readNextSharedFile();
                });
            }, "PlumeLink-share-metadata").start();
        } catch (RuntimeException error) {
            lease.close(); cancelSharedRead();
            shareInbox.metadata(file, "", -1, R.string.ui_share_unreadable);
            emitSharedFiles(false); readNextSharedFile();
        }
    }

    @Override public String getSharedFiles() { return sharedFilesSnapshot; }
    private org.json.JSONObject sharedFilesState(boolean open) {
        org.json.JSONObject state = new org.json.JSONObject();
        try {
            org.json.JSONArray files = new org.json.JSONArray();
            for (ShareInbox.File file : shareInbox.files()) files.put(new org.json.JSONObject()
                .put("id", file.id()).put("name", file.name()).put("size", Math.max(-1, Math.min(file.size(), ResumableDownload.MAX_BYTES + 1))).put("pending", file.pending())
                .put("error", file.error() == 0 ? "" : ui(file.error())));
            state.put("revision", shareInbox.revision()).put("files", files).put("ready", shareInbox.ready())
                .put("message", shareNotice == 0 ? "" : ui(shareNotice)).put("open", open);
        } catch (org.json.JSONException ignored) {}
        return state;
    }
    private void emitSharedFiles(boolean open) {
        if (open) sharedFilesOpenPending = true;
        if (connectUi != null) connectUi.renderSharedFiles();
        sharedFilesSnapshot = sharedFilesState(false).toString();
        WebView source = webView;
        if (source != null && dispatchNativeUi(source, "plumelink-shared-files", sharedFilesState(sharedFilesOpenPending))) {
            sharedFilesOpenPending = false;
        }
    }
    @Override public int sharedFileCount() { return shareInbox.files().size(); }
    @Override public String sharedFileMessage() {
        if (shareNotice != 0) return ui(shareNotice);
        List<ShareInbox.File> files = shareInbox.files();
        if (files.stream().anyMatch(ShareInbox.File::pending)) return ui(R.string.ui_share_reading);
        return files.stream().filter(file -> file.error() != 0).findFirst().map(file -> ui(file.error())).orElse("");
    }
    @Override public void onClearSharedFiles() { clearSharedFiles(shareInbox.revision()); }
    @Override public boolean canRetrySharedFiles() {
        List<ShareInbox.File> files = shareInbox.files();
        return shareReadOwner.isEmpty() && files.stream().noneMatch(ShareInbox.File::pending)
            && files.stream().anyMatch(file -> file.error() != 0);
    }
    @Override public void onRetrySharedFiles() {
        if (!canRetrySharedFiles()) return;
        ShareInbox.File failed = shareInbox.files().stream().filter(file -> file.error() != 0).findFirst().orElse(null);
        if (failed != null) retrySharedFile(shareInbox.revision(), failed.id());
    }
    @Override public void clearSharedFiles(String revision) {
        if (!shareInbox.revision().equals(revision)) return;
        cancelSharedRead();
        sharedFilesOpenPending = false;
        shareInbox.clear(revision); shareNotice = 0; emitSharedFiles(false);
    }
    @Override public void removeSharedFile(String revision, String id) {
        if (!shareInbox.revision().equals(revision)) return;
        if (id.equals(shareReadOwner)) cancelSharedRead();
        shareInbox.remove(revision, id);
        if (shareInbox.empty()) sharedFilesOpenPending = false;
        shareNotice = 0; emitSharedFiles(false); readNextSharedFile();
    }
    @Override public void retrySharedFile(String revision, String id) {
        if (!shareInbox.revision().equals(revision) || isDestroyed()) return;
        ShareInbox.File failed = shareInbox.files().stream().filter(file -> file.id().equals(id) && !file.pending() && file.error() != 0).findFirst().orElse(null);
        if (failed == null) return;
        ProviderTasks.Lease lease = shareReadOwner.isEmpty()
            ? ProviderTasks.SHARED.tryAcquire(java.util.Collections.singleton(ProviderTasks.authority(failed.uri()))) : null;
        if (lease == null) { shareNotice = R.string.ui_file_provider_busy; emitSharedFiles(false); return; }
        ShareInbox.File retried = shareInbox.retry(revision, id);
        if (retried == null) { lease.close(); return; }
        shareNotice = 0;
        readSharedFile(retried, lease);
        emitSharedFiles(false);
    }
    @Override public void sendSharedFiles(WebView source, String revision) {
        if (source != webView || !activityResumed || !shareInbox.revision().equals(revision) || !shareInbox.ready()) return;
        String token = getStoredToken();
        if (session == null || !probingUrl.isEmpty() || !"connected".equals(connectionStatus) || !fileTransferAllowed || !token.matches("^[a-f0-9]{64}$")) {
            shareNotice = R.string.ui_share_connect_first; emitSharedFiles(false); return;
        }
        List<RemoteSessionService.UploadFile> files = shareInbox.files().stream().map(file ->
            new RemoteSessionService.UploadFile(Uri.parse(file.uri()), new ResumableUpload.Metadata(file.name(), file.size()))).collect(java.util.stream.Collectors.toList());
        if (session.upload(currentBaseUrl, expectedHost, store.trustedPin(expectedHost, currentComputerIdentity), token, files)) {
            sharedFilesOpenPending = false;
            shareInbox.consume(revision); shareNotice = 0;
            requestTaskNotifications();
        } else shareNotice = R.string.ui_share_not_started;
        emitSharedFiles(false);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == SCAN_PAIRING) {
            if (resultCode == RESULT_OK && data != null) {
                String contents = com.journeyapps.barcodescanner.ScanIntentResult.parseActivityResult(resultCode, data).getContents();
                String incoming = acceptPairingLink(contents);
                if (!incoming.isEmpty()) connect(incoming);
                else Toast.makeText(this, ui(R.string.ui_scan_a_valid_plumelink_connection_qr_code_from_your_pc), Toast.LENGTH_LONG).show();
            }
            return;
        }
        if (requestCode == PICK_UPLOAD_FILES) {
            android.webkit.ValueCallback<Uri[]> callback = uploadPicker;
            uploadPicker = null;
            if (callback != null) callback.onReceiveValue(null);
            Bundle target = uploadTarget;
            uploadTarget = null;
            if (target == null || target.isEmpty()) return;
            if (resultCode == RESULT_OK && data != null) {
                if (!currentBaseUrl.equals(target.getString("endpoint")) || !currentComputerIdentity.equals(target.getString("identity"))) {
                    Toast.makeText(this, ui(R.string.ui_connect_to_a_pc_first), Toast.LENGTH_SHORT).show();
                    return;
                }
                try { stageSharedFiles(ShareIntents.files(data, false)); }
                catch (RuntimeException error) { shareNotice = R.string.ui_share_invalid; emitSharedFiles(true); }
            }
            return;
        }
        if (requestCode == CONFIRM_SENSITIVE) {
            String action = pendingSensitiveAction;
            WebView source = pendingSensitiveSource == null ? null : pendingSensitiveSource.get();
            pendingSensitiveAction = "";
            pendingSensitiveSource = null;
            if (source != null && source == webView) notifySensitiveAction(action, resultCode == RESULT_OK);
            return;
        }
        if (requestCode != PICK_RECEIVE_TREE) return;
        receivePickerPending = false;
        if (resultCode != RESULT_OK || data == null || data.getData() == null) return;
        Uri tree = data.getData();
        try {
            ReceiveDirectory.select(getContentResolver(), store, tree, data.getFlags());
            unavailableReceiveTree = "";
            notifyTransfer("", receiveFolderLabel());
        } catch (Exception error) {
            notifyTransfer(ui(R.string.ui_could_not_save_folder_permission), receiveFolderLabel());
        }
    }

    private void notifySensitiveAction(String action, boolean approved) {
        if (webView == null || action.isEmpty()) return;
        String detail = "{action:" + org.json.JSONObject.quote(action) + ",approved:" + approved + "}";
        webView.evaluateJavascript("window.dispatchEvent(new CustomEvent('plumelink-native-confirm',{detail:" + detail + "}))", null);
    }

    @Override
    public void onBackPressed() {
        if (webView == null) {
            super.onBackPressed();
            return;
        }
        if (!"connected".equals(connectionStatus)) {
            showConnectScreen(currentBaseUrl, "");
            return;
        }
        WebView source = webView;
        source.evaluateJavascript("Boolean(window.plumeLinkBack && window.plumeLinkBack())", result -> {
            if (webView != source || "true".equals(result)) return;
            showConnectScreen(store.lastUrl(), "");
        });
    }

    @Override public boolean dispatchKeyEvent(KeyEvent event) {
        if (activityResumed && nativeRemoteUi != null && nativeRemoteUi.volumeKey(event)) return true;
        return super.dispatchKeyEvent(event);
    }

    @Override public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        if (!focused && nativeRemoteUi != null) nativeRemoteUi.cancelVolumeKeys();
    }

    @Override
    protected void onPause() {
        cancelCurrentConnectionRecovery();
        inputEpoch = inputEpoch == Integer.MAX_VALUE ? 1 : inputEpoch + 1;
        activityResumed = false;
        if (nativeRemoteUi != null) nativeRemoteUi.releaseVolumeKeys();
        connectUi.setForeground(false);
        discovery.stop();
        if (session != null) session.background(this);
        keepRemoteAwake = false;
        getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        if (webView != null) webView.evaluateJavascript("window.dispatchEvent(new Event('pagehide'))", null);
        super.onPause();
    }

    private void registerNetworkCallback() {
        if (networkRegistered) return;
        ConnectivityManager cm = getSystemService(ConnectivityManager.class);
        if (cm == null) return;
        int generation = ++networkGeneration;
        localNetworks.clear();
        networkCallback = new ConnectivityManager.NetworkCallback() {
            @Override public void onAvailable(Network network) {
                runOnUiThread(() -> {
                    if (isDestroyed() || generation != networkGeneration) return;
                    recoveryNetworkRevision++; cancelCurrentConnectionRecovery();
                    localNetworks.add(network);
                    if (webView == null && hasShownConnectScreen && activityResumed) {
                        connectUi.refreshSavedComputers();
                        if (discovery.isFinished()) discovery.retry();
                    } else if (activityResumed && webView != null && !EndpointUtils.isUsbEndpoint(currentBaseUrl)) {
                        webView.evaluateJavascript("window.dispatchEvent(new Event('online'))", null);
                    }
                });
            }
            @Override public void onLost(Network network) {
                runOnUiThread(() -> {
                    if (isDestroyed() || generation != networkGeneration) return;
                    recoveryNetworkRevision++; cancelCurrentConnectionRecovery();
                    if (!localNetworks.removeLast(network) || !activityResumed) return;
                    if (webView != null && !EndpointUtils.isUsbEndpoint(currentBaseUrl)) {
                        webView.evaluateJavascript("window.dispatchEvent(new Event('offline'))", null);
                    } else if (hasShownConnectScreen) {
                        connectUi.refreshSavedComputers();
                    }
                });
            }
        };
        try {
            NetworkRequest req = new NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET)
                .addTransportType(NetworkCapabilities.TRANSPORT_VPN)
                .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                .build();
            cm.registerNetworkCallback(req, networkCallback);
            networkRegistered = true;
        } catch (Exception ignored) {
            networkCallback = null;
        }
    }

    private void unregisterNetworkCallback() {
        cancelCurrentConnectionRecovery();
        networkGeneration++;
        localNetworks.clear();
        if (!networkRegistered || networkCallback == null) return;
        ConnectivityManager cm = getSystemService(ConnectivityManager.class);
        if (cm != null) {
            try { cm.unregisterNetworkCallback(networkCallback); } catch (Exception ignored) {}
        }
        networkRegistered = false;
        networkCallback = null;
    }

    @Override protected void onStart() { super.onStart(); registerNetworkCallback(); }
    @Override protected void onStop() { unregisterNetworkCallback(); super.onStop(); }

    @Override public void onConfigurationChanged(android.content.res.Configuration config) {
        super.onConfigurationChanged(config);
        if (connectUi == null) return;
        if (webView != null) {
            webView.evaluateJavascript("window.dispatchEvent(new Event('plumelink-appearance'))", null);
        } else if (connectUi.themeChanged()) {
            setSystemBars(AppAppearance.dark(this));
            connectUi.refreshPalette();
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        inputEpoch = inputEpoch == Integer.MAX_VALUE ? 1 : inputEpoch + 1;
        activityResumed = true;
        bindSession();
        if (session != null) session.attachControlHost(this, true);
        refreshReceiveDirectory();
        connectUi.setForeground(true);
        if (session != null) session.snapshot();
        if (webView != null) {
            webView.evaluateJavascript("window.dispatchEvent(new Event('pageshow'))", null);
            emitSharedFiles(false);
        }
        else if (hasShownConnectScreen) discovery.start();
    }

    @Override
    protected void onDestroy() {
        cancelCurrentConnectionRecovery();
        recoveredDraft.clear();
        recoveryProbes.shutdownNow();
        connectionGeneration++;
        cancelConnectionProbe();
        pendingSensitiveAction = "";
        pendingSensitiveSource = null;
        pendingMicrophoneSource = null;
        pendingMicrophoneRequest = 0;
        receiveDirectoryCheck++;
        unregisterNetworkCallback();
        connectUi.destroy();
        if (webClient != null) webClient.clearPrompt();
        if (session != null) {
            if (session.detachControlHost(this)) session.listen(null);
        }
        if (sessionBound) {
            unbindService(sessionConnection);
            sessionBound = false;
        }
        cancelSharedRead();
        discovery.stop();
        destroyWebView(false);
        super.onDestroy();
    }
}
