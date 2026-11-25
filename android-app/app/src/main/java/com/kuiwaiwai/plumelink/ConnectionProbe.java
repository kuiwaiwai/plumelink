package com.kuiwaiwai.plumelink;

import okhttp3.*;
import org.json.JSONObject;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.Proxy;
import java.security.SecureRandom;
import java.security.cert.X509Certificate;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import javax.net.ssl.*;

final class ConnectionProbe {
    enum State { READY, UNREACHABLE, SECURE_FAILED, INVALID_IDENTITY, USB_EXPIRED, CANCELLED }
    record Result(State state, String identity, String pin) {}
    private volatile Call call;
    private volatile boolean cancelled;

    synchronized void cancel() {
        cancelled = true;
        if (call != null) call.cancel();
    }

    @android.annotation.SuppressLint("CustomX509TrustManager")
    Result inspect(String url) { return inspect(url, ""); }
    Result inspect(String url, String usbAccess) {
        if (cancelled) return failure(State.CANCELLED);
        if (!EndpointUtils.isValidPrivateEndpoint(url)) return failure(State.UNREACHABLE);
        try {
            AtomicReference<X509Certificate> peer = new AtomicReference<>();
            // This credential-free probe reads the peer certificate; MainActivity checks saved pins before trusting it.
            X509TrustManager reader = new X509TrustManager() {
                public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
                public void checkClientTrusted(X509Certificate[] chain, String type) throws java.security.cert.CertificateException { throw new java.security.cert.CertificateException(); }
                public void checkServerTrusted(X509Certificate[] chain, String type) throws java.security.cert.CertificateException {
                    if (chain == null || chain.length == 0) throw new java.security.cert.CertificateException();
                    peer.set(chain[0]);
                }
            };
            SSLContext tls = SSLContext.getInstance("TLS");
            tls.init(null, new TrustManager[]{reader}, new SecureRandom());
            String host = new java.net.URI(url).getHost();
            OkHttpClient client = new OkHttpClient.Builder().sslSocketFactory(tls.getSocketFactory(), reader)
                .hostnameVerifier((name, session) -> host != null && name.equals(host.replace("[", "").replace("]", "")))
                .proxy(Proxy.NO_PROXY).connectTimeout(1400, TimeUnit.MILLISECONDS).readTimeout(1400, TimeUnit.MILLISECONDS)
                .callTimeout(4500, TimeUnit.MILLISECONDS).connectionPool(new ConnectionPool(0, 1, TimeUnit.SECONDS))
                .followRedirects(false).followSslRedirects(false).retryOnConnectionFailure(false).build();
            Call request = client.newCall(new Request.Builder().url(url + "/api/identity").header("Accept-Encoding", "identity").get().build());
            synchronized (this) { if (cancelled) return failure(State.CANCELLED); call = request; }
            try (Response response = request.execute()) {
                X509Certificate certificate = peer.get();
                if (response.handshake() == null || certificate == null) return failure(State.SECURE_FAILED);
                String pin = EndpointUtils.certificateFingerprint(certificate);
                if (response.code() == 401 || response.code() == 404) return cancelled ? failure(State.CANCELLED) : EndpointUtils.isUsbEndpoint(url) ? failure(State.USB_EXPIRED) : new Result(State.READY, "", pin);
                ResponseBody body = response.body();
                if (response.code() != 200 || body == null || body.contentLength() > 1024) return failure(State.INVALID_IDENTITY);
                ByteArrayOutputStream content = new ByteArrayOutputStream();
                try (InputStream input = body.byteStream()) {
                    byte[] block = new byte[256]; int count;
                    while ((count = input.read(block, 0, Math.min(block.length, 1025 - content.size()))) != -1) {
                        content.write(block, 0, count);
                        if (content.size() > 1024) return failure(State.INVALID_IDENTITY);
                    }
                }
                String json = content.toString(java.nio.charset.StandardCharsets.UTF_8.name());
                if (EndpointUtils.isUsbEndpoint(url) && !UsbAccess.matchesTag(usbAccess,new JSONObject(json).optString("usbSession"))) return failure(State.USB_EXPIRED);
                String identity = parseIdentity(json);
                if (identity.isEmpty()) return failure(State.INVALID_IDENTITY);
                return cancelled ? failure(State.CANCELLED) : new Result(State.READY, identity, pin);
            }
        } catch (Exception error) {
            return failure(cancelled ? State.CANCELLED : error instanceof SSLException ? State.SECURE_FAILED : State.UNREACHABLE);
        } finally { call = null; }
    }
    static String parseIdentity(String value) {
        if (value == null || value.length() > 1024) return "";
        try {
            JSONObject input = new JSONObject(value);
            Object id = input.opt("id"), version = input.opt("version");
            return "plumelink".equals(input.opt("service")) && version instanceof Number && ((Number)version).doubleValue() == 1
                && id instanceof String && ((String)id).matches("[a-f0-9]{24}") ? (String)id : "";
        } catch (Exception ignored) { return ""; }
    }
    private static Result failure(State state) { return new Result(state, "", ""); }
}
