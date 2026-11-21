package com.kuiwaiwai.plumelink;

import android.net.Uri;
import android.net.http.SslCertificate;
import android.os.Bundle;

import java.net.Inet6Address;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.cert.X509Certificate;
import java.util.Locale;

final class EndpointUtils {
    static final int DEFAULT_SERVICE_PORT = 17840;
    private EndpointUtils() {}

    static String normalize(String input) {
        String value = input.trim().replace(" ", "");
        if (!value.startsWith("https://")) value = "https://" + value;
        String authority = value.substring("https://".length());
        int slash = authority.indexOf('/');
        if (slash >= 0) authority = authority.substring(0, slash);
        int query = authority.indexOf('?');
        if (query >= 0) authority = authority.substring(0, query);
        String explicitPort = null;
        if (authority.startsWith("[")) {
            int close = authority.indexOf(']');
            if (close >= 0 && close + 1 < authority.length() && authority.charAt(close + 1) == ':') {
                explicitPort = authority.substring(close + 2);
            }
        } else {
            int colon = authority.lastIndexOf(':');
            if (colon > 0 && authority.indexOf(':') == colon) explicitPort = authority.substring(colon + 1);
        }
        if (explicitPort != null) {
            if (!explicitPort.matches("[0-9]+") || explicitPort.length() > 5) return value;
            try {
                if (Long.parseLong(explicitPort) > 65535L) return value;
            } catch (NumberFormatException ignored) {
                return value;
            }
        }
        Uri uri = Uri.parse(value);
        String host = uri.getHost();
        if (host == null) return value;
        int parsedPort = uri.getPort();
        if (parsedPort < -1 || parsedPort > 65535) return value;
        int port = parsedPort == -1 ? DEFAULT_SERVICE_PORT : parsedPort;
        String formatted = host.contains(":") && !host.startsWith("[") ? "[" + host + "]" : host;
        return "https://" + formatted + ":" + port;
    }

    static boolean isValidPrivateEndpoint(String value) {
        try {
            Uri uri = Uri.parse(value);
            return "https".equals(uri.getScheme()) && isPrivateHost(uri.getHost())
                && uri.getPort() > 0 && uri.getPort() <= 65535;
        } catch (Exception ignored) {
            return false;
        }
    }

    static boolean isReachable(String url) {
        try (Socket socket = new Socket()) {
            if (!isValidPrivateEndpoint(url)) return false;
            Uri uri = Uri.parse(url);
            socket.connect(new InetSocketAddress(uri.getHost(), uri.getPort()), 1400);
            return true;
        } catch (Exception ignored) {
            return false;
        }
    }

    static boolean isPrivateHost(String host) {
        if (host == null || host.isEmpty()) return false;
        String clean = host;
        if (clean.startsWith("[") && clean.endsWith("]") && clean.length() > 2) {
            clean = clean.substring(1, clean.length() - 1);
        }
        int zone = clean.indexOf('%');
        if (zone > 0) clean = clean.substring(0, zone);
        if (clean.contains(":")) {
            try {
                InetAddress addr = InetAddress.getByName(clean);
                if (addr instanceof Inet6Address) {
                    if (addr.isLoopbackAddress() || addr.isLinkLocalAddress() || addr.isSiteLocalAddress()) return true;
                    byte[] raw = addr.getAddress();
                    return raw.length == 16 && (raw[0] & 0xfe) == 0xfc;
                }
            } catch (Exception ignored) {
                return false;
            }
            return false;
        }
        String[] parts = clean.split("\\.");
        if (parts.length != 4) return false;
        try {
            int a = Integer.parseInt(parts[0]);
            int b = Integer.parseInt(parts[1]);
            for (String part : parts) {
                int value = Integer.parseInt(part);
                if (value < 0 || value > 255) return false;
            }
            return a == 10 || a == 127 || (a == 100 && b >= 64 && b <= 127)
                || a == 192 && b == 168 || a == 172 && b >= 16 && b <= 31 || a == 169 && b == 254;
        } catch (NumberFormatException ignored) {
            return false;
        }
    }

    static String certificateFingerprint(X509Certificate certificate) throws Exception {
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(certificate.getEncoded());
        return formatColonHex(digest);
    }

    static String fingerprint(SslCertificate certificate) {
        try {
            Bundle state = SslCertificate.saveState(certificate);
            byte[] encoded = state == null ? null : state.getByteArray("x509-certificate");
            if (encoded == null) return "";
            return formatColonHex(MessageDigest.getInstance("SHA-256").digest(encoded));
        } catch (Exception ignored) {
            return "";
        }
    }

    static String sha256Hex(String input) {
        try {
            byte[] digest = MessageDigest.getInstance("SHA-256").digest(input.getBytes(StandardCharsets.UTF_8));
            StringBuilder value = new StringBuilder();
            for (byte item : digest) value.append(String.format(Locale.ROOT, "%02x", item));
            return value.toString();
        } catch (Exception ignored) {
            return "fallback";
        }
    }

    static boolean isWindowsDeviceName(String name) {
        String stem = name.split("\\.", 2)[0].trim().toUpperCase(Locale.ROOT);
        if ("CON".equals(stem) || "PRN".equals(stem) || "AUX".equals(stem) || "NUL".equals(stem)
                || "CLOCK$".equals(stem) || "CONIN$".equals(stem) || "CONOUT$".equals(stem)) return true;
        if (stem.length() == 4 && (stem.startsWith("COM") || stem.startsWith("LPT"))) {
            char digit = stem.charAt(3);
            return (digit >= '1' && digit <= '9') || digit == '¹' || digit == '²' || digit == '³';
        }
        return stem.equals("COM¹") || stem.equals("COM²") || stem.equals("COM³")
                || stem.equals("LPT¹") || stem.equals("LPT²") || stem.equals("LPT³");
    }

    private static String formatColonHex(byte[] digest) {
        StringBuilder result = new StringBuilder();
        for (byte value : digest) {
            if (result.length() > 0) result.append(':');
            result.append(String.format(Locale.ROOT, "%02X", value));
        }
        return result.toString();
    }
}
