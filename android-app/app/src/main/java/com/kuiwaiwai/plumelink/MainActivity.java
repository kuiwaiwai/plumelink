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
import android.view.ViewGroup;
import android.view.WindowManager;
import android.webkit.WebChromeClient;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.widget.FrameLayout;
import android.widget.Toast;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public final class MainActivity extends Activity implements ConnectScreenUi.Host, PinnedWebViewClient.Host, AndroidTransferBridge.Host {
    private static final int MIN_WEBVIEW_VERSION = 85;
    private static final int PICK_RECEIVE_TREE = 8127;
    private static final int CONFIRM_SENSITIVE = 8128;
    private static final int PICK_UPLOAD_FILES = 8129;
    private static final int REQUEST_MICROPHONE = 8130;
    private static final int SCAN_PAIRING = 8131;
    private static final String STATE_ACTIVE_PC = "active-pc";
    private static final String STATE_UPLOAD = "upload";
    private static final String STATE_UPLOAD_FILES = "upload-files";
    private static final String STATE_RECEIVE_PICKER = "receive-picker";

    private String ui(int id) { return AppLanguage.text(this, id); }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }

    private FrameLayout root;
    private ComputerStore store;
    private LanDiscovery discovery;
    private ConnectScreenUi connectUi;
    private PinnedWebViewClient webClient;

    private volatile WebView webView;
    private volatile String expectedHost = "";
    private volatile String currentBaseUrl = "";
    private volatile String currentComputerIdentity = "";
    private String pendingPairUrl = "";
    private String pendingPairCode = "";
    private long pendingPairAt;

    private String pendingSensitiveAction = "";
    private boolean keepRemoteAwake;
    private volatile boolean activityResumed;
    private int microphoneRequest;
    private android.webkit.ValueCallback<Uri[]> uploadPicker;
    private Bundle uploadTarget;
    private boolean receivePickerPending;
    private final ExecutorService receiveDirectoryWorker = Executors.newSingleThreadExecutor();
    private volatile int receiveDirectoryCheck;
    private volatile String unavailableReceiveTree = "";
    private ArrayList<Uri> pendingUploadUris;
    private volatile RemoteSessionService session;
    private boolean sessionBound;
    private NativeRemoteUi nativeRemoteUi;
    private boolean connectionKeepAliveRequested;
    private ConnectivityManager.NetworkCallback networkCallback;
    private boolean networkRegistered;
    private boolean hasShownConnectScreen;
    private int connectionGeneration;
    private String probingUrl = "";
    private String connectionStatus = "";
    private Runnable connectionDeadline;

    private final android.content.ServiceConnection sessionConnection = new android.content.ServiceConnection() {
        @Override public void onServiceConnected(android.content.ComponentName name, android.os.IBinder binder) {
            if (isDestroyed() || isFinishing()) return;
            session = ((RemoteSessionService.LocalBinder) binder).service();
            if (connectionKeepAliveRequested && webView != null) session.setConnectionKeepAlive(true);
            String running = session.activeBase();
            if (!running.isEmpty() && webView == null && probingUrl.isEmpty()) connect(running);
            session.listen((event, state) -> {
                if (activityResumed && webView != null && !isDestroyed() && (session.activeBase().isEmpty() || session.activeBase().equals(currentBaseUrl))) {
                    webView.evaluateJavascript("window.dispatchEvent(new CustomEvent(" + org.json.JSONObject.quote(event) + ",{detail:" + state + "}))", null);
                }
                if ("plumelink-native-transfer".equals(event) && state.optBoolean("done") && !state.optBoolean("snapshot")) {
                    refreshReceiveDirectory();
                }
            });
            resumeSelectedUpload();
        }
        @Override public void onServiceDisconnected(android.content.ComponentName name) { session = null; }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        if (Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
        else getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LAYOUT_STABLE | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        setSystemBars(AppAppearance.dark(this));

        root = new FrameLayout(this);
        setContentView(root);
        root.setOnApplyWindowInsetsListener((view, insets) -> {
            if (Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets bars = insets.getInsets(android.view.WindowInsets.Type.systemBars() | android.view.WindowInsets.Type.displayCutout());
                int keyboard = insets.getInsets(android.view.WindowInsets.Type.ime()).bottom;
                view.setPadding(bars.left, bars.top, bars.right, Math.max(bars.bottom, keyboard));
                return android.view.WindowInsets.CONSUMED;
            }
            view.setPadding(insets.getSystemWindowInsetLeft(), insets.getSystemWindowInsetTop(), insets.getSystemWindowInsetRight(), insets.getSystemWindowInsetBottom());
            android.view.WindowInsets remaining = insets.consumeSystemWindowInsets();
            return Build.VERSION.SDK_INT >= 28 ? remaining.consumeDisplayCutout() : remaining;
        });
        root.requestApplyInsets();

        store = new ComputerStore(this);
        if (savedInstanceState != null) {
            uploadTarget = savedInstanceState.getBundle(STATE_UPLOAD);
            pendingUploadUris = savedInstanceState.getParcelableArrayList(STATE_UPLOAD_FILES);
            receivePickerPending = savedInstanceState.getBoolean(STATE_RECEIVE_PICKER);
        }
        discovery = new LanDiscovery(this, () -> {
            if (connectUi != null) connectUi.renderDiscoveredComputers();
        });
        connectUi = new ConnectScreenUi(this, root, store, discovery, this);
        webClient = new PinnedWebViewClient(this, store, this);

        sessionBound = bindService(new Intent(this, RemoteSessionService.class), sessionConnection, BIND_AUTO_CREATE);
        String incoming = savedInstanceState == null ? connectionFromIntent(getIntent())
            : savedInstanceState.getString(STATE_ACTIVE_PC, "");
        if (!incoming.isEmpty()) {
            connect(incoming);
            return;
        }
        connectUi.enableAutoConnectLast(savedInstanceState == null && !store.lastUrl().isEmpty());
        showConnectScreen("", "");
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        String endpoint = !probingUrl.isEmpty() ? probingUrl : webView != null ? currentBaseUrl : "";
        if (!endpoint.isEmpty()) outState.putString(STATE_ACTIVE_PC, endpoint);
        if (uploadTarget != null) outState.putBundle(STATE_UPLOAD, new Bundle(uploadTarget));
        if (pendingUploadUris != null) outState.putParcelableArrayList(STATE_UPLOAD_FILES, pendingUploadUris);
        outState.putBoolean(STATE_RECEIVE_PICKER, receivePickerPending);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        String incoming = connectionFromIntent(intent);
        if (!incoming.isEmpty()) connect(incoming);
    }

    private String connectionFromIntent(Intent intent) {
        return acceptPairingLink(intent == null || intent.getData() == null ? null : intent.getData().toString());
    }

    private synchronized String acceptPairingLink(String contents) {
        pendingPairUrl = "";
        pendingPairCode = "";
        PairingLink link = PairingLink.parse(contents);
        if (link == null) return "";
        String url = EndpointUtils.normalize(link.address);
        if (!EndpointUtils.isValidPrivateEndpoint(url)) return "";
        pendingPairUrl = url;
        pendingPairCode = link.code;
        pendingPairAt = android.os.SystemClock.elapsedRealtime();
        return url;
    }

    @Override public void onConnectRequested(String url) { connect(url); }
    @Override public void onScanRequested() { scanPairingCode(); }
    @Override public void onLanguageSelected(String value) { setLanguagePreference(value); }
    @Override public void onRefreshScreen(String initial, String errorMessage) { showConnectScreen(initial, errorMessage); }
    @Override public boolean isWebViewActive() { return webView != null; }

    @Override public String currentBaseUrl() { return currentBaseUrl; }
    @Override public String expectedHost() { return expectedHost; }
    @Override public String currentComputerIdentity() { return currentComputerIdentity; }
    @Override public boolean isActiveWebView(WebView view) { return view == webView; }
    @Override public void onBindIdentity(String url, String host, String stableId, String pin) { bindIdentity(url, host, stableId, pin); }
    @Override public void onReturnToConnect(String url, String message) { showConnectScreen(url, message); }
    @Override public void onRenderProcessCrashed(String recoverUrl) {
        destroyWebView();
        showConnectScreen(recoverUrl, ui(R.string.ui_page_rendering_failed_returned_to_the_connection_screen));
    }
    @Override public AlertDialog showSecurityDialog(AlertDialog.Builder builder) { return connectUi.showSecurityDialog(builder); }
    @Override public void runOnMain(Runnable task) { runOnUiThread(task); }

    @Override public String getLanguagePreference() { return AppLanguage.preference(this); }
    @Override public void setLanguagePreference(String value) {
        AppLanguage.set(this, value);
        runOnUiThread(() -> { if (session != null) session.languageChanged(); });
    }
    @Override public String getSystemLanguage() { return getResources().getConfiguration().getLocales().toLanguageTags(); }
    @Override public void scanPairingCode() { runOnUiThread(this::launchScanner); }
    @Override public boolean isAwaitingCertificateTrust() { return webClient != null && webClient.isAwaitingTrust(); }
    @Override public synchronized String takePairingCode() {
        String code = currentBaseUrl.equals(pendingPairUrl)
            && android.os.SystemClock.elapsedRealtime() - pendingPairAt < 60000 ? pendingPairCode : "";
        pendingPairUrl = "";
        pendingPairCode = "";
        return code;
    }
    @Override public void startMicrophone(int request) {
        runOnUiThread(() -> {
            if (!activityResumed || webView == null || request <= 0) return;
            microphoneRequest = request;
            WebView target = webView;
            if (session == null) { microphoneEvent(target, request, "error", 0, ui(R.string.ui_background_service_is_starting_try_again)); return; }
            if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
                microphoneEvent(target, request, "permission", 0, "");
                requestPermissions(new String[]{android.Manifest.permission.RECORD_AUDIO}, REQUEST_MICROPHONE);
                return;
            }
            String token = store.token(currentComputerIdentity);
            if (token.isEmpty()) { microphoneEvent(target, request, "error", 0, ui(R.string.ui_connect_to_a_pc_first)); return; }
            session.startMicrophone(currentBaseUrl, expectedHost, store.trustedPin(expectedHost, currentComputerIdentity), token, request);
        });
    }
    @Override public void stopMicrophone(int request) { runOnUiThread(() -> { if (session != null) session.stopMicrophone(request); }); }
    @Override public void lockMicrophone(int request) {
        runOnUiThread(() -> {
            if (session != null && activityResumed && session.lockMicrophone(request)) requestTaskNotifications();
        });
    }
    @Override public void syncSession() { runOnUiThread(() -> { if (session != null) session.snapshot(); }); }
    @Override public String getComputerName() { return store.computerName(currentBaseUrl); }
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
        String model = Build.MODEL == null ? "Android" : Build.MODEL.trim();
        return model.isEmpty() ? ui(R.string.ui_android_device) : model;
    }
    @Override public String getStoredToken() { return store.token(currentComputerIdentity); }
    @Override public void setStoredToken(String value) {
        store.setToken(currentComputerIdentity, value);
        if (value.isEmpty()) runOnUiThread(() -> { if (session != null) session.stopAll(); });
    }
    @Override public String getStoredDeviceId() { return store.deviceId(currentComputerIdentity); }
    @Override public void setStoredDeviceId(String value) { store.setDeviceId(currentComputerIdentity, value); }
    @Override public void showPrivacyPolicy() { runOnUiThread(connectUi::showPrivacyPolicy); }
    @Override public String getKeepAlivePreference() { return store.keepAlivePreference(); }
    @Override public void setKeepAlivePreference(WebView source, boolean enabled) { runOnUiThread(() -> {
        if (source == webView) store.setKeepAlivePreference(enabled);
    }); }
    @Override public void setConnectionKeepAlive(WebView source, boolean enabled) { runOnUiThread(() -> {
        if (source != webView) return;
        connectionKeepAliveRequested = enabled;
        if (session != null) session.setConnectionKeepAlive(connectionKeepAliveRequested);
        if (connectionKeepAliveRequested && activityResumed) requestTaskNotifications();
    }); }
    @Override public String getReceiveFolder() { return receiveFolderLabel(); }
    @Override public boolean hasReceiveFolder() {
        String tree = store.receiveTree();
        return !tree.isEmpty() && !tree.equals(unavailableReceiveTree);
    }
    @Override public void chooseReceiveFolder() {
        runOnUiThread(() -> {
            if (receivePickerPending || uploadTarget != null || !activityResumed || webView == null) return;
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
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
    @Override public void confirmSensitiveAction(String action) {
        runOnUiThread(() -> {
            KeyguardManager keyguard = (KeyguardManager) getSystemService(KEYGUARD_SERVICE);
            Intent intent = keyguard == null ? null : keyguard.createConfirmDeviceCredentialIntent(ui(R.string.ui_confirm_pc_shutdown), ui(R.string.ui_verify_your_identity_to_send_the_shutdown_request));
            if (intent == null) { notifySensitiveAction(action, true); return; }
            pendingSensitiveAction = action;
            try { startActivityForResult(intent, CONFIRM_SENSITIVE); }
            catch (android.content.ActivityNotFoundException ignored) {
                pendingSensitiveAction = "";
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
        getWindow().setStatusBarColor(dark ? Color.rgb(17, 19, 24) : 0xfff7f8fb);
        getWindow().setNavigationBarColor(dark ? Color.rgb(24, 28, 36) : Color.rgb(240, 242, 248));
        if (root != null) root.setBackgroundColor(dark ? Color.rgb(17, 19, 24) : 0xfff7f8fb);
    }

    private void destroyWebView() {
        clearConnectionDeadline();
        connectionStatus = "";
        if (nativeRemoteUi != null) {
            nativeRemoteUi.destroy();
            nativeRemoteUi = null;
        }
        if (webView == null) return;
        connectionKeepAliveRequested = false;
        if (session != null) session.setConnectionKeepAlive(false);
        if (uploadPicker != null) {
            uploadPicker.onReceiveValue(null);
            uploadPicker = null;
        }
        if (webView.getParent() instanceof ViewGroup) ((ViewGroup) webView.getParent()).removeView(webView);
        webView.stopLoading();
        webView.destroy();
        webView = null;
    }

    private void showConnectScreen(String initial, String errorMessage) {
        abandonUpload();
        connectionGeneration++;
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
            return name == null || name.isEmpty() ? "0.1.0" : name;
        } catch (Exception ignored) {
            return "0.1.0";
        }
    }

    private void connect(String url) {
        if (url.equals(probingUrl)) return;
        if (uploadTarget != null && !url.equals(uploadTarget.getString("endpoint"))) abandonUpload();
        connectUi.enableAutoConnectLast(false);
        int generation = ++connectionGeneration;
        probingUrl = url;
        new Thread(() -> {
            boolean available = EndpointUtils.isReachable(url);
            runOnUiThread(() -> {
                if (connectionGeneration != generation || isFinishing() || isDestroyed()) return;
                probingUrl = "";
                if (available) openSession(url);
                else showConnectScreen(url, ui(R.string.ui_pc_unreachable_check_the_service_and_network));
            });
        }, "PlumeLink-connection-check").start();
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

    @SuppressLint({"SetJavaScriptEnabled", "SetTextI18n"})
    private void openSession(String url) {
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
        String discoveredId = discovery.identityFor(url);
        discovery.stop();
        if (session != null && !session.activeBase().isEmpty() && !session.activeBase().equals(url)) session.stopAll();
        Uri uri = Uri.parse(url);
        expectedHost = uri.getHost() == null ? "" : uri.getHost();
        currentBaseUrl = url;
        String storedId = store.storedIdentityOrEmpty(url);
        currentComputerIdentity = discoveredId.matches("^[a-f0-9]{24}$") ? discoveredId
            : (!storedId.isEmpty() ? storedId : url);
        if (discoveredId.matches("^[a-f0-9]{24}$")) {
            String knownPin = store.trustedPin(expectedHost, discoveredId);
            if (!knownPin.isEmpty()) bindIdentity(url, expectedHost, discoveredId, knownPin);
        }
        destroyWebView();
        root.removeAllViews();

        webView = new WebView(this);
        webView.setBackgroundColor(AppAppearance.dark(this) ? 0xff111318 : 0xfff7f8fb);
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
                if ("connected".equals(status) && !"connected".equals(connectionStatus)) store.rememberComputer(currentBaseUrl);
                connectionStatus = status;
                if (nativeRemoteUi == null) nativeRemoteUi = new NativeRemoteUi(this, root, new NativeRemoteUi.Host() {
                    @Override public void action(String id, Object value) {
                        if (source != webView) return;
                        if ("change".equals(id) || "pair-change".equals(id) || "offline-change".equals(id)) {
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
                nativeRemoteUi.render(state);
                if ("unpaired".equals(status) && pendingUploadUris != null && getStoredToken().isEmpty()) abandonUpload();
                resumeSelectedUpload();
            } catch (org.json.JSONException ignored) {}
        });
    }

    private void dispatchNativeUi(WebView source, String type, org.json.JSONObject event) {
        if (source != webView) return;
        source.evaluateJavascript("window.dispatchEvent(new CustomEvent(" + org.json.JSONObject.quote(type)
            + ",{detail:" + event + "}))", null);
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
            Toast.makeText(MainActivity.this, message, Toast.LENGTH_SHORT).show();
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
            String path = DocumentsContract.getTreeDocumentId(tree);
            if ("com.android.externalstorage.documents".equals(tree.getAuthority())) {
                int volumeEnd = path.indexOf(':');
                if (volumeEnd >= 0 && volumeEnd + 1 < path.length()) path = path.substring(volumeEnd + 1);
            }
            return ui(R.string.ui_selected) + path;
        } catch (Exception ignored) {
            return ui(R.string.ui_android_download_folder_selected);
        }
    }

    private void refreshReceiveDirectory() {
        String tree = store.receiveTree();
        int check = ++receiveDirectoryCheck;
        if (tree.isEmpty() || receiveDirectoryWorker.isShutdown()) return;
        receiveDirectoryWorker.execute(() -> {
            if (check != receiveDirectoryCheck) return;
            String unavailable = ReceiveDirectory.writableDocument(getContentResolver(), tree) == null ? tree : "";
            runOnUiThread(() -> {
                if (isDestroyed() || check != receiveDirectoryCheck || !tree.equals(store.receiveTree())
                    || unavailable.equals(unavailableReceiveTree)) return;
                unavailableReceiveTree = unavailable;
                if (webView != null) webView.evaluateJavascript("window.dispatchEvent(new Event('plumelink-native-folder'))", null);
            });
        });
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
        if (code == REQUEST_MICROPHONE && webView != null) {
            boolean granted = results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED;
            boolean retry = shouldShowRequestPermissionRationale(android.Manifest.permission.RECORD_AUDIO);
            boolean shown = nativeRemoteUi != null && nativeRemoteUi.showMicrophonePermissionHint(granted, retry, () -> {
                if (retry) requestPermissions(new String[]{android.Manifest.permission.RECORD_AUDIO}, REQUEST_MICROPHONE);
                else startActivity(new Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                    android.net.Uri.fromParts("package", getPackageName(), null)));
            });
            microphoneEvent(webView, microphoneRequest, "idle", 0, shown ? "" : ui(granted
                ? R.string.ui_microphone_permission_granted_press_and_hold_again : R.string.ui_microphone_permission_denied_enable_it_in_settings));
        }
    }

    private void abandonUpload() {
        if (uploadTarget == null) return;
        // Reserve the request code until the outstanding picker returns.
        uploadTarget = pendingUploadUris == null ? new Bundle() : null;
        pendingUploadUris = null;
    }

    private void resumeSelectedUpload() {
        if (pendingUploadUris == null || session == null || webView == null
            || !"connected".equals(connectionStatus)) return;
        List<Uri> selected = pendingUploadUris;
        Bundle target = uploadTarget;
        pendingUploadUris = null;
        uploadTarget = null;
        String token = store.token(currentComputerIdentity);
        if (!currentBaseUrl.equals(target.getString("endpoint")) || !currentComputerIdentity.equals(target.getString("identity"))
            || !token.matches("^[a-f0-9]{64}$")) {
            notifyTransfer(ui(R.string.ui_connect_to_a_pc_first), receiveFolderLabel());
            return;
        }
        session.upload(currentBaseUrl, expectedHost, store.trustedPin(expectedHost, currentComputerIdentity), token, selected);
        requestTaskNotifications();
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
            ArrayList<Uri> selected = new ArrayList<>();
            Set<String> seenUris = new HashSet<>();
            if (resultCode == RESULT_OK && data != null) {
                if (data.getClipData() != null) {
                    if (data.getClipData().getItemCount() > 16) {
                        Toast.makeText(this, ui(R.string.ui_select_up_to_16_files_at_a_time), Toast.LENGTH_SHORT).show();
                        return;
                    }
                    for (int i = 0; i < Math.min(16, data.getClipData().getItemCount()); i++) {
                        Uri uri = data.getClipData().getItemAt(i).getUri();
                        if (uri != null && "content".equals(uri.getScheme()) && seenUris.add(uri.toString())) selected.add(uri);
                    }
                } else if (data.getData() != null && "content".equals(data.getData().getScheme()) && seenUris.add(data.getData().toString())) {
                    selected.add(data.getData());
                }
            }
            if (!selected.isEmpty()) {
                uploadTarget = target;
                pendingUploadUris = selected;
                resumeSelectedUpload();
            }
            return;
        }
        if (requestCode == CONFIRM_SENSITIVE) {
            String action = pendingSensitiveAction;
            pendingSensitiveAction = "";
            notifySensitiveAction(action, resultCode == RESULT_OK);
            return;
        }
        if (requestCode != PICK_RECEIVE_TREE) return;
        receivePickerPending = false;
        if (resultCode != RESULT_OK || data == null || data.getData() == null) return;
        Uri tree = data.getData();
        try {
            getContentResolver().takePersistableUriPermission(
                tree,
                Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
            );
            store.setReceiveTree(tree.toString());
            unavailableReceiveTree = "";
            notifyTransfer(ui(R.string.ui_android_download_folder_updated), receiveFolderLabel());
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

    @Override
    protected void onPause() {
        connectUi.setForeground(false);
        discovery.stop();
        if (session != null) session.background();
        activityResumed = false;
        keepRemoteAwake = false;
        getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        if (webView != null) webView.evaluateJavascript("window.dispatchEvent(new Event('pagehide'))", null);
        super.onPause();
    }

    private void registerNetworkCallback() {
        if (networkRegistered) return;
        ConnectivityManager cm = getSystemService(ConnectivityManager.class);
        if (cm == null) return;
        networkCallback = new ConnectivityManager.NetworkCallback() {
            @Override public void onAvailable(Network network) {
                runOnUiThread(() -> {
                    if (isDestroyed()) return;
                    if (webView == null && hasShownConnectScreen && activityResumed) {
                        connectUi.refreshSavedComputers();
                        if (discovery.isFinished()) discovery.retry();
                    } else if (activityResumed && webView != null) {
                        webView.evaluateJavascript("window.dispatchEvent(new Event('online'))", null);
                    }
                });
            }
            @Override public void onLost(Network network) {
                runOnUiThread(() -> {
                    if (isDestroyed() || !activityResumed) return;
                    if (webView != null) {
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
                .build();
            cm.registerNetworkCallback(req, networkCallback);
            networkRegistered = true;
        } catch (Exception ignored) {
            networkCallback = null;
        }
    }

    private void unregisterNetworkCallback() {
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
        activityResumed = true;
        refreshReceiveDirectory();
        connectUi.setForeground(true);
        if (session != null) session.snapshot();
        if (webView != null) webView.evaluateJavascript("window.dispatchEvent(new Event('pageshow'))", null);
        else if (hasShownConnectScreen) discovery.start();
    }

    @Override
    protected void onDestroy() {
        receiveDirectoryCheck++;
        receiveDirectoryWorker.shutdownNow();
        unregisterNetworkCallback();
        connectUi.destroy();
        if (webClient != null) webClient.clearPrompt();
        if (session != null) {
            session.setConnectionKeepAlive(false);
            session.background();
            session.listen(null);
        }
        if (sessionBound) {
            unbindService(sessionConnection);
            sessionBound = false;
        }
        pendingUploadUris = null;
        discovery.stop();
        destroyWebView();
        super.onDestroy();
    }
}
