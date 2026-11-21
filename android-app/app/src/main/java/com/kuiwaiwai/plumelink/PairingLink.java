package com.kuiwaiwai.plumelink;

import java.net.URI;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.Map;

/** Parses QR data only; callers must also enforce the private-network policy. */
final class PairingLink {
    final String address;
    final String code;
    private PairingLink(String address, String code) { this.address = address; this.code = code; }

    static PairingLink parse(String value) {
        if (value == null || value.length() > 1024) return null;
        try {
            URI uri = new URI(value);
            if (!"plumelink".equals(uri.getScheme()) || !"connect".equals(uri.getRawAuthority())
                || !(uri.getPath().isEmpty() || "/".equals(uri.getPath())) || uri.getFragment() != null
                || uri.getRawQuery() == null) return null;
            Map<String, String> params = new HashMap<>();
            for (String part : uri.getRawQuery().split("&", -1)) {
                String[] entry = part.split("=", 2);
                if (entry.length != 2) return null;
                String key = URLDecoder.decode(entry[0], StandardCharsets.UTF_8.name());
                String content = URLDecoder.decode(entry[1], StandardCharsets.UTF_8.name());
                if (!("address".equals(key) || "code".equals(key)) || params.put(key, content) != null) return null;
            }
            String address = params.getOrDefault("address", "");
            String code = params.getOrDefault("code", "");
            if (!address.matches("(?:[0-9.]+|\\[[0-9A-Fa-f:.]+\\]):[0-9]{1,5}")) return null;
            URI endpoint = new URI("https://" + address);
            if (endpoint.getHost() == null || endpoint.getPort() < 1 || endpoint.getPort() > 65535) return null;
            if (params.containsKey("code") && !code.matches("[0-9]{6}")) return null;
            return new PairingLink(address, code);
        } catch (Exception ignored) { return null; }
    }
}
