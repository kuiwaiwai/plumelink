package com.kuiwaiwai.plumelink;

import android.content.Context;
import android.content.SharedPreferences;
import android.net.Uri;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.Set;

final class ComputerStore {
    private static final String PREFS = "plumelink_android";
    private static final String LAST_URL = "last_url";
    private static final String SAVED_URLS = "saved_urls";
    private static final String SAVED_ORDER = "saved_order";
    private static final String PINNED_URLS = "pinned_urls";
    private static final String RECEIVE_TREE = "receive_tree";
    private static final String KEEP_ALIVE = "keep_alive";
    static final String ALIAS_PREFIX = "computer_alias_";
    static final String LOCATION_PREFIX = "computer_location_";

    private final Context context;
    private final SharedPreferences prefs;

    ComputerStore(Context context) {
        this.context = context.getApplicationContext();
        this.prefs = this.context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    String lastUrl() { return prefs.getString(LAST_URL, ""); }
    String keepAlivePreference() { return prefs.getString(KEEP_ALIVE, ""); }
    void setKeepAlivePreference(boolean enabled) { prefs.edit().putString(KEEP_ALIVE, Boolean.toString(enabled)).apply(); }
    String receiveTree() { return prefs.getString(RECEIVE_TREE, ""); }
    void setReceiveTree(String uri) { prefs.edit().putString(RECEIVE_TREE, uri).apply(); }
    String knownComputerUrl(String identity) { return prefs.getString("computer_" + identity, ""); }
    String identityForUrl(String url) { return prefs.getString("identity_" + url, url); }
    String storedIdentityOrEmpty(String url) { return prefs.getString("identity_" + url, ""); }

    boolean notificationRequested() { return prefs.getBoolean("notification_requested", false); }
    void markNotificationRequested() { prefs.edit().putBoolean("notification_requested", true).apply(); }
    boolean notificationDeniedPrompted() { return prefs.getBoolean("notification_denied_prompted", false); }
    void markNotificationDeniedPrompted() { prefs.edit().putBoolean("notification_denied_prompted", true).apply(); }

    boolean hasTrustRecord() {
        for (String key : prefs.getAll().keySet()) {
            if (key.startsWith("pin_")) return true;
        }
        return false;
    }

    void resetTrustedCertificates() {
        SharedPreferences.Editor editor = prefs.edit();
        for (String key : prefs.getAll().keySet()) {
            if (key.startsWith("pin_")) editor.remove(key);
        }
        editor.apply();
    }

    String trustedPin(String host, String stableId) {
        if (!stableId.isEmpty()) {
            String identityPin = prefs.getString("pin_id_" + stableId, "");
            if (!identityPin.isEmpty()) return identityPin;
            String previous = prefs.getString("computer_" + stableId, "");
            if (!previous.isEmpty()) {
                String oldPin = prefs.getString("pin_" + Uri.parse(previous).getHost(), "");
                if (!oldPin.isEmpty()) return oldPin;
            }
        }
        return prefs.getString("pin_" + host, "");
    }

    String bindTrustedIdentity(String url, String host, String stableId, String pin) {
        SharedPreferences.Editor editor = prefs.edit().putString("pin_" + host, pin);
        String resolvedIdentity = null;
        if (stableId.matches("^[a-f0-9]{24}$")) {
            String previous = prefs.getString("computer_" + stableId, "");
            editor.putString("pin_id_" + stableId, pin)
                .putString("computer_" + stableId, url)
                .putString("identity_" + url, stableId);
            migrateCredentials(url, stableId, editor);
            migrateMetadata(url, stableId, editor);
            if (!previous.isEmpty() && !previous.equals(url)) {
                migrateCredentials(previous, stableId, editor);
                migrateMetadata(previous, stableId, editor);
                Set<String> saved = new HashSet<>(prefs.getStringSet(SAVED_URLS, Collections.emptySet()));
                saved.remove(previous);
                saved.add(url);
                editor.putStringSet(SAVED_URLS, saved).remove("identity_" + previous);
                if (prefs.contains("used_" + previous)) {
                    editor.putLong("used_" + url, prefs.getLong("used_" + previous, 0)).remove("used_" + previous);
                }
                Set<String> pinned = new HashSet<>(prefs.getStringSet(PINNED_URLS, Collections.emptySet()));
                if (pinned.remove(previous)) pinned.add(url);
                editor.putStringSet(PINNED_URLS, pinned);
                String order = prefs.getString(SAVED_ORDER, "");
                if (!order.isEmpty()) {
                    StringBuilder migratedOrder = new StringBuilder();
                    for (String item : order.split("\\|", -1)) {
                        if (migratedOrder.length() > 0) migratedOrder.append('|');
                        migratedOrder.append(item.equals(previous) ? url : item);
                    }
                    editor.putString(SAVED_ORDER, migratedOrder.toString());
                }
                if (previous.equals(prefs.getString(LAST_URL, ""))) editor.putString(LAST_URL, url);
            }
            resolvedIdentity = stableId;
        }
        editor.apply();
        return resolvedIdentity;
    }

    void rememberComputer(String url) {
        Set<String> saved = new HashSet<>(prefs.getStringSet(SAVED_URLS, Collections.emptySet()));
        saved.add(url);
        prefs.edit().putStringSet(SAVED_URLS, saved).putString(LAST_URL, url)
            .putLong("used_" + url, System.currentTimeMillis()).apply();
    }

    void forgetComputer(String url) {
        Set<String> saved = new HashSet<>(prefs.getStringSet(SAVED_URLS, Collections.emptySet()));
        saved.remove(url);
        String identity = prefs.getString("identity_" + url, url);
        boolean identityStillReferenced = false;
        if (!identity.equals(url)) {
            for (String other : saved) {
                if (identity.equals(prefs.getString("identity_" + other, other))) {
                    identityStillReferenced = true;
                    break;
                }
            }
        }
        SharedPreferences.Editor editor = prefs.edit()
            .putStringSet(SAVED_URLS, saved)
            .remove("used_" + url)
            .remove("identity_" + url)
            .remove(credentialKey(url, "token"))
            .remove(credentialKey(url, "device"))
            .remove(metadataKey(url, ALIAS_PREFIX))
            .remove(metadataKey(url, LOCATION_PREFIX));
        if (!identityStillReferenced) {
            editor.remove(credentialKey(identity, "token"))
                .remove(credentialKey(identity, "device"))
                .remove(metadataKey(identity, ALIAS_PREFIX))
                .remove(metadataKey(identity, LOCATION_PREFIX));
            if (!identity.equals(url)) editor.remove("computer_" + identity);
        }
        if (url.equals(prefs.getString(LAST_URL, ""))) editor.remove(LAST_URL);
        Set<String> pinned = new HashSet<>(prefs.getStringSet(PINNED_URLS, Collections.emptySet()));
        pinned.remove(url);
        ArrayList<String> order = orderedSavedComputers();
        order.remove(url);
        editor.putStringSet(PINNED_URLS, pinned).putString(SAVED_ORDER, String.join("|", order)).apply();
    }

    ArrayList<String> visibleSavedComputers() {
        ArrayList<String> result = orderedSavedComputers();
        result.sort((left, right) -> Boolean.compare(isPinned(right), isPinned(left)));
        return result;
    }

    private ArrayList<String> orderedSavedComputers() {
        Set<String> saved = prefs.getStringSet(SAVED_URLS, Collections.emptySet());
        ArrayList<String> result = new ArrayList<>();
        String encoded = prefs.getString(SAVED_ORDER, "");
        if (!encoded.isEmpty()) {
            for (String url : encoded.split("\\|")) {
                if (!url.isEmpty() && saved.contains(url) && !result.contains(url)) result.add(url);
            }
        }
        ArrayList<String> missing = new ArrayList<>(saved);
        missing.removeAll(result);
        missing.sort((left, right) -> Long.compare(prefs.getLong("used_" + right, 0), prefs.getLong("used_" + left, 0)));
        result.addAll(missing);
        return result;
    }

    boolean isPinned(String url) {
        return prefs.getStringSet(PINNED_URLS, Collections.emptySet()).contains(url);
    }

    void togglePinned(String url) {
        Set<String> pinned = new HashSet<>(prefs.getStringSet(PINNED_URLS, Collections.emptySet()));
        if (!pinned.add(url)) pinned.remove(url);
        prefs.edit().putStringSet(PINNED_URLS, pinned).apply();
    }

    void moveSavedComputer(String url, int direction) {
        ArrayList<String> urls = visibleSavedComputers();
        int index = urls.indexOf(url);
        int target = index + direction;
        if (index < 0 || target < 0 || target >= urls.size()) return;
        Collections.swap(urls, index, target);
        prefs.edit().putString(SAVED_ORDER, String.join("|", urls)).apply();
    }

    String computerMetadata(String url, String prefix) {
        return prefs.getString(metadataKey(identityForUrl(url), prefix), "");
    }

    String computerName(String url) {
        String alias = computerMetadata(url, ALIAS_PREFIX).trim();
        return alias.isEmpty() ? AppLanguage.text(context, R.string.ui_windows_pc) : alias;
    }

    String computerSubtitle(String url) {
        String location = computerMetadata(url, LOCATION_PREFIX).trim();
        String address = url.replace("https://", "");
        return location.isEmpty() ? address : address + "\n" + location;
    }

    void saveComputerMetadata(String url, String alias, String location) {
        String identity = identityForUrl(url);
        String aliasKey = metadataKey(identity, ALIAS_PREFIX);
        String locationKey = metadataKey(identity, LOCATION_PREFIX);
        SharedPreferences.Editor editor = prefs.edit();
        String cleanAlias = alias.replaceAll("\\p{Cntrl}", "").trim();
        String cleanLocation = location.replaceAll("\\p{Cntrl}", "").trim();
        if (cleanAlias.isEmpty()) editor.remove(aliasKey); else editor.putString(aliasKey, cleanAlias);
        if (cleanLocation.isEmpty()) editor.remove(locationKey); else editor.putString(locationKey, cleanLocation);
        editor.apply();
    }

    String token(String identity) { return prefs.getString(credentialKey(identity, "token"), ""); }

    void setToken(String identity, String value) {
        SharedPreferences.Editor editor = prefs.edit();
        if (value.isEmpty()) editor.remove(credentialKey(identity, "token"));
        else editor.putString(credentialKey(identity, "token"), value);
        editor.apply();
    }

    String deviceId(String identity) { return prefs.getString(credentialKey(identity, "device"), ""); }

    void setDeviceId(String identity, String value) {
        prefs.edit().putString(credentialKey(identity, "device"), value).apply();
    }

    private String metadataKey(String identity, String prefix) {
        return prefix + EndpointUtils.sha256Hex(identity);
    }

    private String credentialKey(String identity, String kind) {
        return "credential_" + kind + "_" + EndpointUtils.sha256Hex(identity);
    }

    private void migrateMetadata(String sourceIdentity, String targetIdentity, SharedPreferences.Editor editor) {
        String sourceAliasKey = metadataKey(sourceIdentity, ALIAS_PREFIX);
        String sourceLocationKey = metadataKey(sourceIdentity, LOCATION_PREFIX);
        String sourceAlias = prefs.getString(sourceAliasKey, "");
        String sourceLocation = prefs.getString(sourceLocationKey, "");
        String targetAliasKey = metadataKey(targetIdentity, ALIAS_PREFIX);
        String targetLocationKey = metadataKey(targetIdentity, LOCATION_PREFIX);
        if (!sourceAlias.isEmpty() && !prefs.contains(targetAliasKey)) editor.putString(targetAliasKey, sourceAlias);
        if (!sourceLocation.isEmpty() && !prefs.contains(targetLocationKey)) editor.putString(targetLocationKey, sourceLocation);
        if (!sourceIdentity.equals(targetIdentity)) editor.remove(sourceAliasKey).remove(sourceLocationKey);
    }

    private void migrateCredentials(String url, String stableId, SharedPreferences.Editor editor) {
        String oldTokenKey = credentialKey(url, "token");
        String oldDeviceKey = credentialKey(url, "device");
        String oldToken = prefs.getString(oldTokenKey, "");
        String oldDevice = prefs.getString(oldDeviceKey, "");
        if (!oldToken.isEmpty() && !prefs.contains(credentialKey(stableId, "token"))) {
            editor.putString(credentialKey(stableId, "token"), oldToken);
        }
        if (!oldDevice.isEmpty() && !prefs.contains(credentialKey(stableId, "device"))) {
            editor.putString(credentialKey(stableId, "device"), oldDevice);
        }
        if (!url.equals(stableId)) editor.remove(oldTokenKey).remove(oldDeviceKey);
    }
}
