package com.kuiwaiwai.plumelink;

import android.webkit.JavascriptInterface;
import android.webkit.WebView;

import java.nio.charset.StandardCharsets;

final class AndroidTransferBridge {
    interface Host {
        BridgeAccess<WebView> bridgeAccess();
        String getLanguagePreference();
        void renderNativeUi(WebView source, String state);
        void setLanguagePreference(String value);
        String getSystemLanguage();
        void scanPairingCode();
        boolean isAwaitingCertificateTrust();
        String takePairingCode(WebView source);
        void startMicrophone(int request);
        default boolean hasMicrophonePermission() { return false; }
        void stopMicrophone(int request);
        void lockMicrophone(int request);
        void syncSession();
        String getComputerName();
        void haptic();
        void setKeepAwake(boolean enabled);
        void showPrivacyPolicy();
        String getKeepAlivePreference();
        void setKeepAlivePreference(WebView source, boolean enabled);
        void setConnectionKeepAlive(WebView source, boolean enabled);
        default String getConnectionKeepAliveState() { return "{}"; }
        default boolean isControlForegroundReady() { return true; }
        default void openBackgroundSettings() {}
        void setImmersive(boolean enabled);
        boolean isSystemDark();
        String getThemePreference();
        void setThemePreference(String value);
        void setAppearance(boolean dark);
        String getDeviceName();
        default String getDeviceModel() { return ""; }
        String getStoredToken();
        default String getUsbAccessKey() { return ""; }
        default int getInputEpoch() { return 0; }
        void setStoredToken(String value);
        String getStoredDeviceId();
        void setStoredDeviceId(String value);
        String getReceiveFolder();
        boolean hasReceiveFolder();
        String getMobilePreferences();
        void setMobilePreferences(String value);
        void refreshReceiveFolder();
        void chooseReceiveFolder();
        void chooseUploadFiles();
        default String getSharedFiles() { return ""; }
        default void clearSharedFiles(String revision) {}
        default void removeSharedFile(String revision, String id) {}
        default void retrySharedFile(String revision, String id) {}
        default void sendSharedFiles(WebView source, String revision) {}
        void changeComputer();
        default void recoverConnection(WebView source, String draft, String request) {}
        default void cancelConnectionRecovery(WebView source, String request) {}
        default String takeRecoveredDraft(WebView source) { return ""; }
        void startDownload(String path, String fileName, String token);
        void cancelFileTransfer();
        void resolveUnfinishedDownload(String id, String version, boolean keep, String request);
        void confirmSensitiveAction(String action);
        void notifyInvalidDownload();
        void notifyInvalidFileName();
    }

    private final Host host;
    private final WebView source;
    private final BridgeAccess<WebView> access;

    AndroidTransferBridge(Host host, WebView source) {
        this.host = host;
        this.source = source;
        access = host.bridgeAccess();
    }

    @JavascriptInterface public String getLanguagePreference() { return access.read(source, host::getLanguagePreference, "system"); }
    @JavascriptInterface public void renderNativeUi(String state) {
        if (state != null && state.length() <= 262144) access.run(source, () -> host.renderNativeUi(source, state));
    }
    @JavascriptInterface public void setLanguagePreference(String value) { access.run(source, () -> host.setLanguagePreference(value)); }
    @JavascriptInterface public String getSystemLanguage() { return access.read(source, host::getSystemLanguage, "en"); }
    @JavascriptInterface public void scanPairingCode() { access.run(source, host::scanPairingCode); }
    @JavascriptInterface public boolean isAwaitingCertificateTrust() { return access.read(source, host::isAwaitingCertificateTrust, false); }
    @JavascriptInterface public String takePairingCode() { return access.read(source, () -> host.takePairingCode(source), ""); }
    @JavascriptInterface public void startMicrophone(int request) { access.run(source, () -> host.startMicrophone(request)); }
    @JavascriptInterface public boolean hasMicrophonePermission() { return access.read(source, host::hasMicrophonePermission, false); }
    @JavascriptInterface public void stopMicrophone(int request) { access.run(source, () -> host.stopMicrophone(request)); }
    @JavascriptInterface public void lockMicrophone(int request) { access.run(source, () -> host.lockMicrophone(request)); }
    @JavascriptInterface public void syncSession() { access.run(source, host::syncSession); }
    @JavascriptInterface public String getComputerName() { return access.read(source, host::getComputerName, ""); }
    @JavascriptInterface public void haptic() { access.run(source, host::haptic); }
    @JavascriptInterface public void setKeepAwake(boolean enabled) { access.run(source, () -> host.setKeepAwake(enabled)); }
    @JavascriptInterface public void showPrivacyPolicy() { access.run(source, host::showPrivacyPolicy); }
    @JavascriptInterface public String getKeepAlivePreference() { return access.read(source, host::getKeepAlivePreference, ""); }
    @JavascriptInterface public void setKeepAlivePreference(boolean enabled) { access.run(source, () -> host.setKeepAlivePreference(source, enabled)); }
    @JavascriptInterface public void setConnectionKeepAlive(boolean enabled) { access.run(source, () -> host.setConnectionKeepAlive(source, enabled)); }
    @JavascriptInterface public String getConnectionKeepAliveState() { return access.read(source, host::getConnectionKeepAliveState, "{}"); }
    @JavascriptInterface public boolean isControlForegroundReady() { return access.read(source, host::isControlForegroundReady, false); }
    @JavascriptInterface public void openBackgroundSettings() { access.run(source, host::openBackgroundSettings); }
    @JavascriptInterface public void setImmersive(boolean enabled) { access.run(source, () -> host.setImmersive(enabled)); }
    @JavascriptInterface public boolean isSystemDark() { return access.read(source, host::isSystemDark, false); }
    @JavascriptInterface public String getThemePreference() { return access.read(source, host::getThemePreference, "system"); }
    @JavascriptInterface public void setThemePreference(String value) { access.run(source, () -> host.setThemePreference(value)); }
    @JavascriptInterface public void setAppearance(boolean dark) { access.run(source, () -> host.setAppearance(dark)); }
    @JavascriptInterface public String getDeviceName() { return access.read(source, host::getDeviceName, "Android"); }
    @JavascriptInterface public String getDeviceModel() { return access.read(source, host::getDeviceModel, ""); }
    @JavascriptInterface public String getStoredToken() { return access.read(source, host::getStoredToken, ""); }
    @JavascriptInterface public String getUsbAccessKey() { return access.read(source, host::getUsbAccessKey, ""); }
    @JavascriptInterface public int getInputEpoch() { return access.read(source, host::getInputEpoch, 0); }
    @JavascriptInterface public void setStoredToken(String value) {
        if (value == null || !(value.isEmpty() || value.matches("^[a-f0-9]{64}$"))) return;
        access.run(source, () -> host.setStoredToken(value));
    }
    @JavascriptInterface public String getStoredDeviceId() { return access.read(source, host::getStoredDeviceId, ""); }
    @JavascriptInterface public void setStoredDeviceId(String value) {
        if (value == null || !value.matches("^[A-Za-z0-9_-]{8,64}$")) return;
        access.run(source, () -> host.setStoredDeviceId(value));
    }
    @JavascriptInterface public String getReceiveFolder() { return access.read(source, host::getReceiveFolder, ""); }
    @JavascriptInterface public boolean hasReceiveFolder() { return access.read(source, host::hasReceiveFolder, false); }
    @JavascriptInterface public String getMobilePreferences() { return access.read(source, host::getMobilePreferences, ""); }
    @JavascriptInterface public void setMobilePreferences(String value) {
        String normalized = MobilePreferences.incoming(value);
        if (!normalized.isEmpty()) access.run(source, () -> host.setMobilePreferences(normalized));
    }
    @JavascriptInterface public void refreshReceiveFolder() { access.run(source, host::refreshReceiveFolder); }
    @JavascriptInterface public void chooseReceiveFolder() { access.run(source, host::chooseReceiveFolder); }
    @JavascriptInterface public void chooseUploadFiles() { access.run(source, host::chooseUploadFiles); }
    @JavascriptInterface public String getSharedFiles() { return access.read(source, host::getSharedFiles, ""); }
    @JavascriptInterface public void clearSharedFiles(String revision) {
        if (validShareId(revision)) access.run(source, () -> host.clearSharedFiles(revision));
    }
    @JavascriptInterface public void removeSharedFile(String revision, String id) {
        if (validShareId(revision) && validShareId(id)) access.run(source, () -> host.removeSharedFile(revision, id));
    }
    @JavascriptInterface public void retrySharedFile(String revision, String id) {
        if (validShareId(revision) && validShareId(id)) access.run(source, () -> host.retrySharedFile(revision, id));
    }
    @JavascriptInterface public void sendSharedFiles(String revision) {
        if (validShareId(revision)) access.run(source, () -> host.sendSharedFiles(source, revision));
    }
    private static boolean validShareId(String value) { return value != null && value.matches("[a-f0-9]{32}"); }
    @JavascriptInterface public void changeComputer() { access.run(source, host::changeComputer); }
    @JavascriptInterface public void recoverConnection(String draft, String request) {
        if (LanRecovery.validDraft(draft) && validShareId(request)) access.run(source, () -> host.recoverConnection(source, draft, request));
    }
    @JavascriptInterface public void cancelConnectionRecovery(String request) {
        if (validShareId(request)) access.run(source, () -> host.cancelConnectionRecovery(source, request));
    }
    @JavascriptInterface public String takeRecoveredDraft() { return access.read(source, () -> host.takeRecoveredDraft(source), ""); }
    @JavascriptInterface public void cancelFileTransfer() { access.run(source, host::cancelFileTransfer); }
    @JavascriptInterface public void resolveUnfinishedDownload(String id, String version, boolean keep, String request) {
        if (id == null || !id.matches("[a-f0-9]{32}") || version == null || !(version.isEmpty() || version.matches("[a-f0-9]{64}"))
            || request == null || !request.matches("[a-f0-9]{32}")) return;
        access.run(source, () -> host.resolveUnfinishedDownload(id, version, keep, request));
    }
    @JavascriptInterface public void confirmSensitiveAction(String action) {
        if ("shutdown".equals(action)) access.run(source, () -> host.confirmSensitiveAction(action));
    }

    @JavascriptInterface
    public void downloadFile(String path, String fileName, String token) {
        if (path == null || fileName == null || token == null) return;
        if (!path.matches("^/api/files/[0-9]+$") || !token.matches("^[a-f0-9]{64}$")) {
            access.run(source, host::notifyInvalidDownload);
            return;
        }
        if (fileName.isEmpty() || fileName.getBytes(StandardCharsets.UTF_8).length > 240 || fileName.equals(".") || fileName.equals("..")
                || fileName.indexOf('\\') >= 0 || fileName.indexOf('/') >= 0
                || fileName.endsWith(".") || fileName.endsWith(" ")
                || fileName.startsWith(".plumelink-")
                || fileName.chars().anyMatch(ch -> ch < 0x20 || ch == 0x7f)
                || fileName.chars().anyMatch(ch -> "<>:\"|?*".indexOf(ch) >= 0)
                || EndpointUtils.isWindowsDeviceName(fileName)) {
            access.run(source, host::notifyInvalidFileName);
            return;
        }
        access.run(source, () -> host.startDownload(path, fileName, token));
    }
}
