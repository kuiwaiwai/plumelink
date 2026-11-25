package com.kuiwaiwai.plumelink;

import java.net.URL;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.security.cert.X509Certificate;
import java.util.Locale;
import javax.net.ssl.*;
import okhttp3.ConnectionPool;
import okhttp3.OkHttpClient;
import okhttp3.HttpUrl;
import okhttp3.Request;
import java.util.concurrent.TimeUnit;

/** Trust only the certificate and host approved for the selected computer. */
final class PinnedConnections {
    private static final ConnectionPool TRANSFER_POOL = new ConnectionPool(4, 30, TimeUnit.SECONDS);
    final SSLSocketFactory sockets;
    final X509TrustManager trust;
    final HostnameVerifier hostname;
    private static final String USB_HEADER = "X-PlumeLink-USB";
    private final HttpUrl usbBase;
    private final String usbKey;
    PinnedConnections(String host, String pin) throws Exception { this(host, pin, "", ""); }
    @android.annotation.SuppressLint("CustomX509TrustManager") // Self-signed LAN certificates are accepted only by exact user-approved SHA-256 pin and host.
    PinnedConnections(String host, String pin, String base, String usbKey) throws Exception {
        if (pin == null || pin.isEmpty()) throw new Exception("尚未信任电脑证书");
        HttpUrl parsed = HttpUrl.parse(base);
        usbBase = parsed != null && "https".equals(parsed.scheme()) && "127.0.0.1".equals(parsed.host())
            && host.equals(parsed.host()) && parsed.username().isEmpty() && parsed.password().isEmpty() ? parsed : null;
        this.usbKey = usbBase != null && UsbAccess.validKey(usbKey) ? usbKey : "";
        trust = new X509TrustManager() {
            public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
            public void checkClientTrusted(X509Certificate[] chain, String type) throws java.security.cert.CertificateException { throw new java.security.cert.CertificateException("不接受客户端证书"); }
            public void checkServerTrusted(X509Certificate[] chain, String type) throws java.security.cert.CertificateException {
                try {
                    if (chain == null || chain.length == 0 || !pin.equals(fingerprint(chain[0]))) throw new java.security.cert.CertificateException("电脑证书不匹配");
                } catch (java.security.cert.CertificateException e) { throw e; }
                catch (Exception e) { throw new java.security.cert.CertificateException(e); }
            }
        };
        SSLContext ssl = SSLContext.getInstance("TLS"); ssl.init(null, new TrustManager[]{trust}, new SecureRandom()); sockets = ssl.getSocketFactory();
        hostname = (name, session) -> {
            if (!host.equals(name)) return false;
            try { return pin.equals(fingerprint((X509Certificate) session.getPeerCertificates()[0])); }
            catch (Exception e) { return false; }
        };
    }
    private boolean acceptsUsb(HttpUrl target) {
        return usbBase != null && !usbKey.isEmpty() && "https".equals(target.scheme())
            && usbBase.host().equals(target.host()) && usbBase.port() == target.port();
    }
    Request authorizeUsb(Request request) {
        Request.Builder builder = request.newBuilder().removeHeader(USB_HEADER);
        if (acceptsUsb(request.url())) builder.header(USB_HEADER, usbKey);
        return builder.build();
    }
    static String fingerprint(X509Certificate certificate) throws Exception {
        StringBuilder out = new StringBuilder();
        for (byte b : MessageDigest.getInstance("SHA-256").digest(certificate.getEncoded())) {
            if (out.length() > 0) out.append(':'); out.append(String.format(Locale.ROOT, "%02X", b));
        }
        return out.toString();
    }
    HttpsURLConnection open(String url, String token) throws Exception {
        HttpsURLConnection result = (HttpsURLConnection) new URL(url).openConnection();
        result.setSSLSocketFactory(sockets); result.setHostnameVerifier(hostname);
        result.setConnectTimeout(8000); result.setReadTimeout(15000); result.setInstanceFollowRedirects(false);
        HttpUrl target = HttpUrl.parse(url);
        if (target != null && acceptsUsb(target)) result.setRequestProperty(USB_HEADER, usbKey);
        if (!token.isEmpty()) result.setRequestProperty("Authorization", "Bearer " + token);
        result.setRequestProperty("Accept-Encoding", "identity");
        return result;
    }
    OkHttpClient audioClient() {
        return new OkHttpClient.Builder().sslSocketFactory(sockets, trust).hostnameVerifier(hostname)
            .addInterceptor(chain -> chain.proceed(authorizeUsb(chain.request())))
            .connectTimeout(5, TimeUnit.SECONDS).readTimeout(5, TimeUnit.SECONDS).writeTimeout(2, TimeUnit.SECONDS)
            .pingInterval(1, TimeUnit.SECONDS).followRedirects(false).followSslRedirects(false).retryOnConnectionFailure(false).build();
    }
    OkHttpClient transferClient() {
        return new OkHttpClient.Builder().sslSocketFactory(sockets, trust).hostnameVerifier(hostname)
            .addInterceptor(chain -> chain.proceed(authorizeUsb(chain.request())))
            .connectionPool(TRANSFER_POOL).connectTimeout(8, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS).writeTimeout(30, TimeUnit.SECONDS)
            .followRedirects(false).followSslRedirects(false).retryOnConnectionFailure(false).build();
    }
}
