package com.kuiwaiwai.plumelink;

import android.annotation.SuppressLint;
import android.app.AlertDialog;
import android.content.Context;
import android.net.Uri;
import android.net.http.SslError;
import android.webkit.RenderProcessGoneDetail;
import android.webkit.SslErrorHandler;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import java.io.ByteArrayInputStream;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

final class PinnedWebViewClient extends WebViewClient {
    interface Host {
        String currentBaseUrl();
        String expectedHost();
        String currentComputerIdentity();
        boolean isActiveWebView(WebView view);
        void onBindIdentity(String url, String host, String stableId, String pin);
        void onReturnToConnect(String url, String message);
        default void onVerifyComputer(String url) { onReturnToConnect(url, ""); }
        void onRenderProcessCrashed(String recoverUrl);
        AlertDialog showSecurityDialog(AlertDialog.Builder builder);
        void runOnMain(Runnable task);
    }

    private final Context context;
    private final ComputerStore store;
    private final Host host;

    static final class SslPrompt {
        final WebView source;
        final String baseUrl, expectedHost, stableId, pin;
        final boolean awaitingTrust;
        final List<SslErrorHandler> handlers = new ArrayList<>();
        AlertDialog dialog;

        SslPrompt(WebView source, String baseUrl, String expectedHost, String stableId, String pin, boolean awaitingTrust) {
            this.source = source;
            this.baseUrl = baseUrl;
            this.expectedHost = expectedHost;
            this.stableId = stableId;
            this.pin = pin;
            this.awaitingTrust = awaitingTrust;
        }
    }

    private volatile SslPrompt activePrompt;

    PinnedWebViewClient(Context context, ComputerStore store, Host host) {
        this.context = context;
        this.store = store;
        this.host = host;
    }

    private String ui(int id) { return AppLanguage.text(context, id); }

    boolean isAwaitingTrust() {
        SslPrompt prompt = activePrompt;
        return prompt != null && prompt.awaitingTrust;
    }

    void clearPrompt() {
        SslPrompt previous = activePrompt;
        activePrompt = null;
        if (previous == null) return;
        for (SslErrorHandler pending : previous.handlers) {
            try { pending.cancel(); } catch (Exception ignored) {}
        }
        previous.handlers.clear();
        if (previous.dialog != null) {
            try { previous.dialog.dismiss(); } catch (Exception ignored) {}
        }
    }

    SslPrompt beginPrompt(WebView source, String baseUrl, String expectedHost, String stableId, String pin, boolean awaitingTrust) {
        clearPrompt();
        SslPrompt prompt = new SslPrompt(source, baseUrl, expectedHost, stableId, pin, awaitingTrust);
        activePrompt = prompt;
        return prompt;
    }

    private boolean ownsPrompt(SslPrompt prompt) {
        return activePrompt == prompt && host.isActiveWebView(prompt.source);
    }

    void trustPrompt(SslPrompt prompt) {
        if (!ownsPrompt(prompt) || !prompt.awaitingTrust) return;
        activePrompt = null;
        host.onBindIdentity(prompt.baseUrl, prompt.expectedHost, prompt.stableId, prompt.pin);
        for (SslErrorHandler pending : prompt.handlers) {
            try { pending.proceed(); } catch (Exception ignored) {}
        }
        prompt.handlers.clear();
    }

    void returnFromPrompt(SslPrompt prompt, String message) {
        if (!ownsPrompt(prompt)) return;
        clearPrompt();
        if (host.isActiveWebView(prompt.source)) host.onReturnToConnect(prompt.baseUrl, message);
    }

    private static int httpsPort(Uri uri) {
        return uri.getPort() == -1 ? 443 : uri.getPort();
    }

    @Override
    public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
        Uri target = request.getUrl();
        String baseUrl = host.currentBaseUrl();
        Uri server = Uri.parse(baseUrl);
        if (server.getHost() == null || !"https".equals(target.getScheme())
            || !server.getHost().equals(target.getHost())
            || httpsPort(server) != httpsPort(target)) return null;
        String path = target.getPath();
        String asset;
        if ("/".equals(path) || "/index.html".equals(path)) {
            asset = "index.html";
        } else if (path != null && path.matches("^/assets/[A-Za-z0-9_.-]+\\.(js|css|svg|png|woff2|wasm)$")) {
            asset = path.substring(1);
        } else {
            return null;
        }
        try {
            String mime = asset.endsWith(".html") ? "text/html"
                : asset.endsWith(".js") ? "text/javascript"
                : asset.endsWith(".wasm") ? "application/wasm"
                : asset.endsWith(".css") ? "text/css"
                : asset.endsWith(".svg") ? "image/svg+xml"
                : asset.endsWith(".png") ? "image/png" : "font/woff2";
            WebResourceResponse response = new WebResourceResponse(mime, "UTF-8", context.getAssets().open(asset));
            Map<String, String> headers = new HashMap<>();
            headers.put("Cache-Control", "no-store");
            headers.put("X-Content-Type-Options", "nosniff");
            if (asset.equals("index.html")) {
                String cspHost = target.getHost() != null && target.getHost().contains(":") && !target.getHost().startsWith("[")
                    ? "[" + target.getHost() + "]" : target.getHost();
                headers.put("Content-Security-Policy", "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self' wss://" + cspHost + ":" + httpsPort(target) + "; img-src 'self' data:; font-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'");
            }
            response.setResponseHeaders(headers);
            return response;
        } catch (Exception ignored) {
            return new WebResourceResponse("text/plain", "UTF-8", 404, "Not Found", Collections.emptyMap(), new ByteArrayInputStream(new byte[0]));
        }
    }

    @Override
    public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
        Uri target = request.getUrl();
        Uri server = Uri.parse(host.currentBaseUrl());
        return !"https".equals(target.getScheme())
            || !host.expectedHost().equals(target.getHost())
            || httpsPort(server) != httpsPort(target);
    }

    @Override
    public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) {
        if (!host.isActiveWebView(view) || !request.isForMainFrame()) return;
        String failed = host.currentBaseUrl();
        int code = error.getErrorCode();
        String detail = code == ERROR_HOST_LOOKUP ? ui(R.string.ui_could_not_resolve_the_pc_address_scan_again_or_enter_th)
            : code == ERROR_CONNECT ? ui(R.string.ui_pc_unreachable_check_the_service_and_network)
            : code == ERROR_TIMEOUT ? ui(R.string.ui_pc_connection_timed_out_check_the_network_and_retry)
            : code == ERROR_FAILED_SSL_HANDSHAKE ? ui(R.string.ui_secure_connection_failed_verify_the_pc_and_its_certific)
            : ui(R.string.ui_could_not_open_the_pc_page_check_the_service_and_networ);
        host.runOnMain(() -> {
            if (host.isActiveWebView(view)) host.onReturnToConnect(failed, detail);
        });
    }

    @Override
    public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
        if (!host.isActiveWebView(view)) return true;
        String recoverUrl = host.currentBaseUrl();
        host.runOnMain(() -> {
            if (host.isActiveWebView(view)) host.onRenderProcessCrashed(recoverUrl);
        });
        return true;
    }

    @Override
    @SuppressLint("WebViewClientOnReceivedSslError")
    public void onReceivedSslError(WebView view, SslErrorHandler handler, SslError error) {
        if (!host.isActiveWebView(view)) {
            handler.cancel();
            return;
        }
        String baseUrl = host.currentBaseUrl();
        String expected = host.expectedHost();
        Uri uri = Uri.parse(error.getUrl());
        String current = EndpointUtils.fingerprint(error.getCertificate());
        // A paired client can open its secure WebSocket before its first HTTPS request.
        boolean secureTransport = "https".equals(uri.getScheme()) || "wss".equals(uri.getScheme());
        if (!secureTransport || !expected.equals(uri.getHost()) || httpsPort(Uri.parse(baseUrl)) != httpsPort(uri)
            || !EndpointUtils.isPrivateHost(uri.getHost()) || current.isEmpty()) {
            handler.cancel();
            host.onReturnToConnect(baseUrl, ui(R.string.ui_invalid_certificate_connection_blocked_select_the_pc_or));
            return;
        }
        String identity = host.currentComputerIdentity();
        String stableId = identity.matches("^[a-f0-9]{24}$") ? identity : store.storedIdentityOrEmpty(baseUrl);
        String saved = store.trustedPin(expected, stableId);
        if (saved.equals(current)) {
            host.onBindIdentity(baseUrl, expected, stableId, current);
            handler.proceed();
            return;
        }
        if (!saved.isEmpty()) {
            handler.cancel();
            host.onVerifyComputer(baseUrl);
            return;
        }
        SslPrompt previous = activePrompt;
        if (previous != null && previous.dialog != null && previous.dialog.isShowing()) {
            if (previous.source == view && previous.awaitingTrust && baseUrl.equals(previous.baseUrl) && current.equals(previous.pin)) previous.handlers.add(handler);
            else handler.cancel();
            return;
        }
        SslPrompt prompt = beginPrompt(view, baseUrl, expected, stableId, current, true);
        prompt.handlers.add(handler);
        prompt.dialog = host.showSecurityDialog(AppAppearance.dialog(context)
            .setTitle(ui(R.string.ui_trust_this_pc))
            .setMessage(ui(R.string.ui_verify_that_this_address_matches_your_pc) + expected + ui(R.string.ui_sha_256_certificate_fingerprint) + current)
            .setNegativeButton(ui(R.string.ui_cancel), (dialog, which) ->
                returnFromPrompt(prompt, ui(R.string.ui_trust_canceled_choose_another_pc)))
            .setPositiveButton(ui(R.string.ui_trust_and_connect), (dialog, which) -> trustPrompt(prompt))
            .setOnCancelListener(dialog ->
                returnFromPrompt(prompt, ui(R.string.ui_trust_canceled_choose_another_pc)))
        );
    }
}
