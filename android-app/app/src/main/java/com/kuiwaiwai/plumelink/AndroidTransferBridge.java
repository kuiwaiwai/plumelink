package com.kuiwaiwai.plumelink;

import android.webkit.JavascriptInterface;
import android.webkit.WebView;

import java.nio.charset.StandardCharsets;

final class AndroidTransferBridge {
    interface Host {
        String getLanguagePreference();
        void renderNativeUi(WebView source, String state);
        void setLanguagePreference(String value);
        String getSystemLanguage();
        void scanPairingCode();
        boolean isAwaitingCertificateTrust();
        String takePairingCode();
        void startMicrophone(int request);
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
        void setImmersive(boolean enabled);
        boolean isSystemDark();
        String getThemePreference();
        void setThemePreference(String value);
        void setAppearance(boolean dark);
        String getDeviceName();
        String getStoredToken();
        void setStoredToken(String value);
        String getStoredDeviceId();
        void setStoredDeviceId(String value);
        String getReceiveFolder();
        boolean hasReceiveFolder();
        void chooseReceiveFolder();
        void chooseUploadFiles();
        void changeComputer();
        void startDownload(String path, String fileName, String token);
        void cancelFileTransfer();
        void confirmSensitiveAction(String action);
        void notifyInvalidDownload();
        void notifyInvalidFileName();
    }

    private final Host host;
    private final WebView source;

    AndroidTransferBridge(Host host, WebView source) {
        this.host = host;
        this.source = source;
    }

    @JavascriptInterface public String getLanguagePreference() { return host.getLanguagePreference(); }
    @JavascriptInterface public void renderNativeUi(String state) {
        if (state != null && state.length() <= 262144) host.renderNativeUi(source, state);
    }
    @JavascriptInterface public void setLanguagePreference(String value) { host.setLanguagePreference(value); }
    @JavascriptInterface public String getSystemLanguage() { return host.getSystemLanguage(); }
    @JavascriptInterface public void scanPairingCode() { host.scanPairingCode(); }
    @JavascriptInterface public boolean isAwaitingCertificateTrust() { return host.isAwaitingCertificateTrust(); }
    @JavascriptInterface public String takePairingCode() { return host.takePairingCode(); }
    @JavascriptInterface public void startMicrophone(int request) { host.startMicrophone(request); }
    @JavascriptInterface public void stopMicrophone(int request) { host.stopMicrophone(request); }
    @JavascriptInterface public void lockMicrophone(int request) { host.lockMicrophone(request); }
    @JavascriptInterface public void syncSession() { host.syncSession(); }
    @JavascriptInterface public String getComputerName() { return host.getComputerName(); }
    @JavascriptInterface public void haptic() { host.haptic(); }
    @JavascriptInterface public void setKeepAwake(boolean enabled) { host.setKeepAwake(enabled); }
    @JavascriptInterface public void showPrivacyPolicy() { host.showPrivacyPolicy(); }
    @JavascriptInterface public String getKeepAlivePreference() { return host.getKeepAlivePreference(); }
    @JavascriptInterface public void setKeepAlivePreference(boolean enabled) { host.setKeepAlivePreference(source, enabled); }
    @JavascriptInterface public void setConnectionKeepAlive(boolean enabled) { host.setConnectionKeepAlive(source, enabled); }
    @JavascriptInterface public void setImmersive(boolean enabled) { host.setImmersive(enabled); }
    @JavascriptInterface public boolean isSystemDark() { return host.isSystemDark(); }
    @JavascriptInterface public String getThemePreference() { return host.getThemePreference(); }
    @JavascriptInterface public void setThemePreference(String value) { host.setThemePreference(value); }
    @JavascriptInterface public void setAppearance(boolean dark) { host.setAppearance(dark); }
    @JavascriptInterface public String getDeviceName() { return host.getDeviceName(); }
    @JavascriptInterface public String getStoredToken() { return host.getStoredToken(); }
    @JavascriptInterface public void setStoredToken(String value) {
        if (value == null || !(value.isEmpty() || value.matches("^[a-f0-9]{64}$"))) return;
        host.setStoredToken(value);
    }
    @JavascriptInterface public String getStoredDeviceId() { return host.getStoredDeviceId(); }
    @JavascriptInterface public void setStoredDeviceId(String value) {
        if (value == null || !value.matches("^[A-Za-z0-9_-]{8,64}$")) return;
        host.setStoredDeviceId(value);
    }
    @JavascriptInterface public String getReceiveFolder() { return host.getReceiveFolder(); }
    @JavascriptInterface public boolean hasReceiveFolder() { return host.hasReceiveFolder(); }
    @JavascriptInterface public void chooseReceiveFolder() { host.chooseReceiveFolder(); }
    @JavascriptInterface public void chooseUploadFiles() { host.chooseUploadFiles(); }
    @JavascriptInterface public void changeComputer() { host.changeComputer(); }
    @JavascriptInterface public void cancelFileTransfer() { host.cancelFileTransfer(); }
    @JavascriptInterface public void confirmSensitiveAction(String action) {
        if ("shutdown".equals(action)) host.confirmSensitiveAction(action);
    }

    @JavascriptInterface
    public void downloadFile(String path, String fileName, String token) {
        if (path == null || fileName == null || token == null) return;
        if (!path.matches("^/api/files/[0-9]+$") || !token.matches("^[a-f0-9]{64}$")) {
            host.notifyInvalidDownload();
            return;
        }
        if (fileName.isEmpty() || fileName.getBytes(StandardCharsets.UTF_8).length > 240 || fileName.equals(".") || fileName.equals("..")
                || fileName.indexOf('\\') >= 0 || fileName.indexOf('/') >= 0
                || fileName.endsWith(".") || fileName.endsWith(" ")
                || fileName.startsWith(".plumelink-")
                || fileName.chars().anyMatch(ch -> ch < 0x20 || ch == 0x7f)
                || fileName.chars().anyMatch(ch -> "<>:\"|?*".indexOf(ch) >= 0)
                || EndpointUtils.isWindowsDeviceName(fileName)) {
            host.notifyInvalidFileName();
            return;
        }
        host.startDownload(path, fileName, token);
    }
}
