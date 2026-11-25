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
    private static final String USB_SAVED = "saved_usb_computers";
    private static final String SAVED_IDENTITIES = "saved_computer_identities";
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
    String lastIdentity() { return prefs.getString("last_computer_identity", ""); }
    String endpointFor(String record) {
        if (!record.matches("[a-f0-9]{24}")) return record;
        String wireless = prefs.getString("computer_" + record, ""), usb = prefs.getString("usb_" + record, "");
        return (!usb.isEmpty() && prefs.getBoolean("prefer_usb_" + record, false)) || wireless.isEmpty() ? usb : wireless;
    }
    String usbEndpoint(String record) { return prefs.getString("usb_" + identityForUrl(record), ""); }
    String usbAccess(String endpoint) { return UsbAccess.get(prefs.getString("usb_access", ""),endpoint); }
    synchronized void setUsbAccess(String endpoint,String key) { prefs.edit().putString("usb_access",UsbAccess.put(prefs.getString("usb_access",""),endpoint,key)).apply(); }
    String wirelessEndpoint(String record) { return prefs.getString("computer_" + identityForUrl(record), ""); }
    void preferUsb(String record, boolean usb) { prefs.edit().putBoolean("prefer_usb_" + identityForUrl(record), usb).apply(); }
    String keepAlivePreference() { return prefs.getString(KEEP_ALIVE, ""); }
    void setKeepAlivePreference(boolean enabled) { prefs.edit().putString(KEEP_ALIVE, Boolean.toString(enabled)).apply(); }
    String mobilePreferences(String url) {
        return mobilePreferencesForIdentity(identityForUrl(url));
    }
    private String mobilePreferencesForIdentity(String identity) {
        String value = MobilePreferences.normalize(prefs.getString(metadataKey(identity, "mobile_settings_"), ""));
        if (!value.isEmpty()) return value;
        String legacy = ShortcutPreferences.normalize(prefs.getString(metadataKey(identity, "shortcuts_"), ""));
        return legacy.isEmpty() ? "" : "{\"shortcuts\":" + legacy + "}";
    }
    void setMobilePreferences(String url, String value) {
        String normalized = MobilePreferences.incoming(value);
        if (normalized.isEmpty()) return;
        String identity = identityForUrl(url);
        prefs.edit().putString(metadataKey(identity, "mobile_settings_"), normalized).remove(metadataKey(identity, "shortcuts_")).apply();
    }
    boolean speechProcessing(String url) {
        try { return "voice".equals(new org.json.JSONObject(mobilePreferences(url)).optString("microphoneMode")); }
        catch (org.json.JSONException ignored) { return false; }
    }
    String receiveTree() { return prefs.getString(RECEIVE_TREE, ""); }
    void setReceiveTree(String uri) { prefs.edit().putString(RECEIVE_TREE, uri).apply(); }
    String knownComputerUrl(String identity) { String wireless = prefs.getString("computer_" + identity, ""); return wireless.isEmpty() ? prefs.getString("usb_" + identity, "") : wireless; }
    String identityForUrl(String url) { return prefs.getString("identity_" + url, url); }
    String storedIdentityOrEmpty(String url) { return url.matches("[a-f0-9]{24}") ? url : prefs.getString("identity_" + url, ""); }
    String trustedIdentity(String pin) {
        String result = "";
        for (java.util.Map.Entry<String, ?> item : prefs.getAll().entrySet()) {
            if (!item.getKey().matches("pin_id_[a-f0-9]{24}") || !pin.equals(item.getValue())) continue;
            String id = item.getKey().substring("pin_id_".length());
            if (!result.isEmpty() && !result.equals(id)) return "";
            result = id;
        }
        return result;
    }
    private String legacyComputerForPin(String pin) {
        String found = "";
        for (String url : visibleSavedComputers()) {
            if (!storedIdentityOrEmpty(url).isEmpty()) continue;
            try {
                String host = new java.net.URI(url).getHost();
                if (host == null || !pin.equals(prefs.getString("pin_" + host, ""))) continue;
                if (!found.isEmpty() && !found.equals(url)) return "";
                found = url;
            } catch (Exception ignored) {}
        }
        return found;
    }

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
        return "127.0.0.1".equals(host) ? "" : prefs.getString("pin_" + host, "");
    }

    String bindTrustedIdentity(String url, String host, String stableId, String pin) {
        Set<String> savedIdentities = savedIdentities();
        SharedPreferences.Editor editor = prefs.edit().putStringSet(SAVED_IDENTITIES, savedIdentities).putStringSet(PINNED_URLS, normalizedPinned());
        preserveEndpointOwner(url, stableId, editor);
        if (EndpointUtils.isUsbEndpoint(url) && stableId.matches("[a-f0-9]{24}")) {
            String previousOwner = prefs.getString("identity_" + url, "");
            String previousUsb = prefs.getString("usb_" + stableId, "");
            editor.putString("pin_id_" + stableId, pin)
                .putString("usb_" + stableId, url).putString("identity_" + url, stableId);
            if (!previousUsb.isEmpty() && !previousUsb.equals(url) && stableId.equals(storedIdentityOrEmpty(previousUsb))) {
                editor.remove("identity_" + previousUsb).putString("usb_access",UsbAccess.put(prefs.getString("usb_access",""),previousUsb,""));
            }
            if (previousOwner.equals(stableId)) {
                migrateMobilePreferences(stableId, "", url, editor); migrateCredentials(url, stableId, editor); migrateMetadata(url, stableId, editor);
            } else if (previousOwner.isEmpty()) {
                editor.remove(credentialKey(url,"token")).remove(credentialKey(url,"device"))
                    .remove(metadataKey(url,"mobile_settings_")).remove(metadataKey(url,"shortcuts_"))
                    .remove(metadataKey(url,ALIAS_PREFIX)).remove(metadataKey(url,LOCATION_PREFIX));
            }
            editor.apply(); return stableId;
        }
        editor.putString("pin_" + host, pin);
        String resolvedIdentity = null;
        if (stableId.matches("^[a-f0-9]{24}$")) {
            String previous = prefs.getString("computer_" + stableId, "");
            if (previous.isEmpty()) previous = legacyComputerForPin(pin);
            String previousIdentity = prefs.getString("identity_" + url, "");
            if (!previousIdentity.isEmpty() && !previousIdentity.equals(stableId)) {
                Set<String> saved = new HashSet<>(prefs.getStringSet(SAVED_URLS, Collections.emptySet()));
                saved.remove(url);
                editor.putStringSet(SAVED_URLS, saved);
            }
            migrateMobilePreferences(stableId, previous, previousIdentity.isEmpty() || previousIdentity.equals(stableId) ? url : "", editor);
            editor.putString("pin_id_" + stableId, pin)
                .putString("computer_" + stableId, url)
                .putString("identity_" + url, stableId);
            migrateCredentials(url, stableId, editor);
            migrateMetadata(url, stableId, editor);
            String previousOwner = prefs.getString("identity_" + previous, "");
            if (!previous.isEmpty() && !previous.equals(url) && (previousOwner.isEmpty() || previousOwner.equals(stableId))) {
                migrateCredentials(previous, stableId, editor);
                migrateMetadata(previous, stableId, editor);
                Set<String> saved = new HashSet<>(prefs.getStringSet(SAVED_URLS, Collections.emptySet()));
                saved.remove(previous);
                saved.add(url);
                editor.putStringSet(SAVED_URLS, saved).remove("identity_" + previous);
                if (prefs.contains("used_" + previous)) {
                    editor.putLong("used_" + url, prefs.getLong("used_" + previous, 0)).remove("used_" + previous);
                }
                Set<String> pinned = normalizedPinned();
                if (pinned.remove(previous)) pinned.add(stableId);
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
        rememberComputer(url, storedIdentityOrEmpty(url));
    }
    void rememberComputer(String url, String identity) {
        if (!identity.isEmpty() && (!identity.matches("[a-f0-9]{24}") || !identity.equals(storedIdentityOrEmpty(url)))) return;
        Set<String> identities = savedIdentities();
        if (!identity.isEmpty()) identities.add(identity);
        if (EndpointUtils.isUsbEndpoint(url) && !identity.isEmpty()) {
            Set<String> saved = new HashSet<>(prefs.getStringSet(USB_SAVED, Collections.emptySet())); saved.add(identity);
            prefs.edit().putStringSet(SAVED_IDENTITIES, identities).putStringSet(USB_SAVED, saved).putString(LAST_URL, url).putString("last_computer_identity", identity)
                .putBoolean("prefer_usb_" + identity, true).putLong("used_" + identity, System.currentTimeMillis()).apply(); return;
        }
        Set<String> saved = new HashSet<>(prefs.getStringSet(SAVED_URLS, Collections.emptySet()));
        saved.add(url);
        prefs.edit().putStringSet(SAVED_IDENTITIES, identities).putStringSet(SAVED_URLS, saved).putString(LAST_URL, url)
            .putString("last_computer_identity", identity).putBoolean("prefer_usb_" + identity, false)
            .putLong("used_" + identity, System.currentTimeMillis())
            .putLong("used_" + url, System.currentTimeMillis()).apply();
    }

    void prepareTrustReplacement(String url, String previousIdentity, String nextIdentity) {
        String owner = storedIdentityOrEmpty(url);
        String previous = owner.isEmpty() ? previousIdentity.isEmpty() ? identityForUrl(url) : previousIdentity : owner;
        Set<String> targets = new HashSet<>();
        for (String identity : new String[]{previous, nextIdentity, url}) {
            if (identity.isEmpty()) continue;
            targets.add(identity);
            String wireless = wirelessEndpoint(identity), usb = usbEndpoint(identity);
            if (!wireless.isEmpty()) targets.add(wireless);
            if (!usb.isEmpty()) targets.add(usb);
        }
        SharedPreferences.Editor editor = prefs.edit();
        for (String target : targets) editor.remove(credentialKey(target, "token")).remove(credentialKey(target, "device"));
        editor.apply();
        if (!previous.equals(nextIdentity) && previous.matches("[a-f0-9]{24}")) forgetComputer(previous);
    }

    void forgetComputer(String url) {
        String identity = identityForUrl(url);
        String wireless = wirelessEndpoint(identity), usb = usbEndpoint(identity);
        if (!identity.matches("[a-f0-9]{24}")) { forgetWirelessComputer(url); return; }
        Set<String> identities = savedIdentities(); identities.remove(identity);
        Set<String> owned = new HashSet<>(prefs.getStringSet(SAVED_URLS, Collections.emptySet()));
        if (!wireless.isEmpty()) owned.add(wireless);
        for (String endpoint : owned) if (identity.equals(storedIdentityOrEmpty(endpoint))) forgetWirelessComputer(endpoint);
        Set<String> saved = new HashSet<>(prefs.getStringSet(USB_SAVED, Collections.emptySet())); saved.remove(identity);
        SharedPreferences.Editor editor = prefs.edit().putStringSet(SAVED_IDENTITIES, identities).putStringSet(USB_SAVED, saved).remove("usb_" + identity).remove("prefer_usb_" + identity)
            .remove("used_" + identity).remove("computer_" + identity)
            .remove(credentialKey(identity,"token")).remove(credentialKey(identity,"device"))
            .remove(metadataKey(identity,"mobile_settings_")).remove(metadataKey(identity,"shortcuts_"))
            .remove(metadataKey(identity,ALIAS_PREFIX)).remove(metadataKey(identity,LOCATION_PREFIX));
        if (identity.equals(storedIdentityOrEmpty(usb))) editor.remove("identity_" + usb).putString("usb_access",UsbAccess.put(prefs.getString("usb_access",""),usb,""));
        if (identity.equals(lastIdentity())) editor.remove(LAST_URL).remove("last_computer_identity");
        Set<String> pinned = new HashSet<>(prefs.getStringSet(PINNED_URLS,Collections.emptySet())); pinned.remove(identity); editor.putStringSet(PINNED_URLS,pinned);
        ArrayList<String> order = new ArrayList<>();
        for (String item : prefs.getString("saved_record_order", "").split("\\|")) if (!item.isEmpty() && !item.equals(identity) && !item.equals(wireless)) order.add(item);
        editor.putString("saved_record_order",String.join("|",order));
        editor.apply();
    }
    private void forgetWirelessComputer(String url) {
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
            .remove(metadataKey(url, "shortcuts_"))
            .remove(metadataKey(url, "mobile_settings_"))
            .remove(credentialKey(url, "token"))
            .remove(credentialKey(url, "device"))
            .remove(metadataKey(url, ALIAS_PREFIX))
            .remove(metadataKey(url, LOCATION_PREFIX));
        if (!identityStillReferenced) {
            editor.remove(credentialKey(identity, "token"))
                .remove(credentialKey(identity, "device"))
                .remove(metadataKey(identity, "mobile_settings_"))
                .remove(metadataKey(identity, "shortcuts_"))
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
    ArrayList<String> visibleSavedRecords() {
        ArrayList<String> records = new ArrayList<>();
        for (String url : visibleSavedComputers()) {
            String identity = storedIdentityOrEmpty(url), record = identity.isEmpty() ? url : identity;
            if (!records.contains(record)) records.add(record);
        }
        ArrayList<String> usb = new ArrayList<>(savedIdentities());
        usb.sort((a,b)->Long.compare(prefs.getLong("used_"+b,0),prefs.getLong("used_"+a,0)));
        for (String identity : usb) if (identity.matches("[a-f0-9]{24}") && !records.contains(identity)) records.add(identity);
        records.sort((a,b)->Long.compare(lastUsed(b),lastUsed(a)));
        ArrayList<String> ordered = new ArrayList<>();
        for (String item : prefs.getString("saved_record_order", prefs.getString(SAVED_ORDER, "")).split("\\|")) {
            String record = identityForUrl(item); if (records.contains(record) && !ordered.contains(record)) ordered.add(record);
        }
        for (String record : records) if (!ordered.contains(record)) ordered.add(record);
        ordered.sort((a,b)->Boolean.compare(isPinned(b),isPinned(a))); return ordered;
    }
    private Set<String> savedIdentities() {
        Set<String> identities = new HashSet<>(prefs.getStringSet(SAVED_IDENTITIES, Collections.emptySet()));
        identities.addAll(prefs.getStringSet(USB_SAVED, Collections.emptySet()));
        for (String url : prefs.getStringSet(SAVED_URLS, Collections.emptySet())) {
            String identity = storedIdentityOrEmpty(url);
            if (identity.matches("[a-f0-9]{24}")) identities.add(identity);
        }
        identities.removeIf(identity -> !identity.matches("[a-f0-9]{24}"));
        return identities;
    }
    private long lastUsed(String record) {
        String endpoint = wirelessEndpoint(record);
        return Math.max(prefs.getLong("used_" + record, 0), record.equals(storedIdentityOrEmpty(endpoint)) ? prefs.getLong("used_" + endpoint, 0) : 0);
    }
    private Set<String> normalizedPinned() {
        Set<String> result = new HashSet<>();
        for (String record : prefs.getStringSet(PINNED_URLS, Collections.emptySet())) result.add(identityForUrl(record));
        return result;
    }
    private void preserveEndpointOwner(String url, String target, SharedPreferences.Editor editor) {
        String owner = storedIdentityOrEmpty(url);
        if (!owner.matches("[a-f0-9]{24}") || owner.equals(target)) return;
        migrateMobilePreferences(owner, "", url, editor);
        migrateCredentials(url, owner, editor);
        migrateMetadata(url, owner, editor);
        editor.putLong("used_" + owner, Math.max(prefs.getLong("used_" + owner, 0), prefs.getLong("used_" + url, 0))).remove("used_" + url);
        ArrayList<String> order = new ArrayList<>();
        for (String item : prefs.getString("saved_record_order", prefs.getString(SAVED_ORDER, "")).split("\\|")) {
            String record = identityForUrl(item);
            if (!record.isEmpty() && !order.contains(record)) order.add(record);
        }
        if (!order.isEmpty()) editor.putString("saved_record_order", String.join("|", order));
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
        Set<String> pinned = prefs.getStringSet(PINNED_URLS, Collections.emptySet());
        return pinned.contains(url) || (url.matches("[a-f0-9]{24}") && pinned.contains(wirelessEndpoint(url)));
    }

    void togglePinned(String url) {
        String identity = identityForUrl(url); boolean wasPinned = isPinned(url);
        Set<String> pinned = new HashSet<>(prefs.getStringSet(PINNED_URLS, Collections.emptySet()));
        pinned.remove(url); pinned.remove(identity); pinned.remove(wirelessEndpoint(identity));
        if (!wasPinned) pinned.add(identity);
        prefs.edit().putStringSet(PINNED_URLS, pinned).apply();
    }

    void moveSavedComputer(String url, int direction) {
        ArrayList<String> urls = visibleSavedRecords();
        int index = urls.indexOf(url);
        int target = index + direction;
        if (index < 0 || target < 0 || target >= urls.size()) return;
        Collections.swap(urls, index, target);
        prefs.edit().putString("saved_record_order", String.join("|", urls)).apply();
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
        String endpoint = endpointFor(url);
        String address = EndpointUtils.isUsbEndpoint(endpoint) ? AppLanguage.text(context,R.string.ui_usb_connection) : endpoint.replace("https://", "");
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

    private void migrateMobilePreferences(String target, String previous, String current, SharedPreferences.Editor editor) {
        String previousOwner = prefs.getString("identity_" + previous, "");
        if (!previousOwner.isEmpty() && !previousOwner.equals(target)) previous = "";
        String chosen = "";
        for (String candidate : new String[]{target, previous, current}) {
            if (!candidate.isEmpty()) chosen = mobilePreferencesForIdentity(candidate);
            if (!chosen.isEmpty()) break;
        }
        if (!chosen.isEmpty()) {
            try {
                org.json.JSONObject value = new org.json.JSONObject(chosen);
                if (value.has("version")) editor.putString(metadataKey(target, "mobile_settings_"), chosen).remove(metadataKey(target, "shortcuts_"));
                else editor.putString(metadataKey(target, "shortcuts_"), value.getJSONArray("shortcuts").toString());
            } catch (org.json.JSONException ignored) {}
        }
        for (String source : new String[]{previous, current}) {
            if (!source.isEmpty() && !source.equals(target)) editor.remove(metadataKey(source, "mobile_settings_")).remove(metadataKey(source, "shortcuts_"));
        }
    }

    private void migrateMetadata(String sourceIdentity, String targetIdentity, SharedPreferences.Editor editor) {
        String owner = storedIdentityOrEmpty(sourceIdentity);
        if (!owner.isEmpty() && !owner.equals(targetIdentity)) return;
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
        String owner = storedIdentityOrEmpty(url);
        if (!owner.isEmpty() && !owner.equals(stableId)) return;
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
