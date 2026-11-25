package com.kuiwaiwai.plumelink;

import java.net.URI;
import org.json.JSONArray;

final class UsbAccess {
    static final int LIMIT = 128;
    private UsbAccess() {}
    static boolean validKey(String key) { return key != null && key.matches("[a-f0-9]{64}"); }
    static String tag(String key) { return validKey(key) ? EndpointUtils.sha256Hex(key) : ""; }
    static boolean matchesTag(String key, String tag) { return validKey(key) && tag != null && tag.matches("[a-f0-9]{64}") && tag(key).equals(tag); }
    static boolean validEndpoint(String endpoint) {
        try {
            URI uri = new URI(endpoint);
            return "https".equals(uri.getScheme()) && "127.0.0.1".equals(uri.getHost()) && uri.getPort() > 0 && uri.getPort() <= 65535
                && uri.getRawUserInfo() == null && uri.getRawQuery() == null && uri.getRawFragment() == null && uri.getRawPath().isEmpty();
        } catch (Exception ignored) { return false; }
    }
    private static JSONArray entries(String value) {
        try { return value != null && value.length() <= 32768 ? new JSONArray(value) : new JSONArray(); }
        catch (Exception ignored) { return new JSONArray(); }
    }
    static String get(String persisted, String endpoint) {
        if (!validEndpoint(endpoint)) return "";
        JSONArray items = entries(persisted);
        for (int i = Math.max(0, items.length() - LIMIT); i < items.length(); i++) {
            JSONArray item = items.optJSONArray(i);
            if (item != null && endpoint.equals(item.optString(0)) && validKey(item.optString(1))) return item.optString(1);
        }
        return "";
    }
    static String put(String persisted, String endpoint, String key) {
        if (!validEndpoint(endpoint) || !(key != null && key.isEmpty() || validKey(key))) return persisted;
        JSONArray items = entries(persisted), next = new JSONArray();
        for (int i = Math.max(0, items.length() - LIMIT + (key.isEmpty() ? 0 : 1)); i < items.length(); i++) {
            JSONArray item = items.optJSONArray(i);
            if (item != null && validEndpoint(item.optString(0)) && validKey(item.optString(1)) && !endpoint.equals(item.optString(0))) next.put(item);
        }
        if (!key.isEmpty()) next.put(new JSONArray().put(endpoint).put(key));
        return next.toString();
    }
}
