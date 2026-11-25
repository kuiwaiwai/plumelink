package com.kuiwaiwai.plumelink;

import android.webkit.WebView;
import java.lang.reflect.Proxy;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;
import static org.junit.Assert.*;

public class PinnedWebViewClientTest {
    private static final class Fixture {
        final WebView first = new WebView(null), second = new WebView(null);
        final AtomicReference<WebView> current = new AtomicReference<>(first);
        final List<Runnable> pending = new ArrayList<>();
        final List<String> effects = new ArrayList<>();
        final PinnedWebViewClient.Host host = (PinnedWebViewClient.Host) Proxy.newProxyInstance(
            PinnedWebViewClient.Host.class.getClassLoader(), new Class<?>[]{PinnedWebViewClient.Host.class}, (proxy, method, args) -> {
                switch (method.getName()) {
                    case "isActiveWebView": return current.get() == args[0];
                    case "currentBaseUrl": return "https://127.0.0.1:17840";
                    case "runOnMain": pending.add((Runnable) args[0]); return null;
                    default: effects.add(method.getName()); return null;
                }
            });
        final PinnedWebViewClient client = new PinnedWebViewClient(null, null, host);
        void flush() { List<Runnable> work = new ArrayList<>(pending); pending.clear(); work.forEach(Runnable::run); }
    }

    @Test public void queuedRendererFailureCannotReplaceANewerConnection() {
        Fixture f = new Fixture();
        assertTrue(f.client.onRenderProcessGone(f.first, null));
        f.current.set(f.second); f.flush();
        assertTrue(f.effects.isEmpty());
        assertTrue(f.client.onRenderProcessGone(f.first, null));
        assertTrue(f.pending.isEmpty());
    }

    @Test public void currentRendererFailureStillRecovers() {
        Fixture f = new Fixture();
        assertTrue(f.client.onRenderProcessGone(f.first, null)); f.flush();
        assertEquals(Collections.singletonList("onRenderProcessCrashed"), f.effects);
    }

    private static PinnedWebViewClient.SslPrompt prompt(Fixture f, WebView source, boolean awaitingTrust) {
        return f.client.beginPrompt(source, "https://127.0.0.1:17840", "127.0.0.1", "a".repeat(24), "b".repeat(64), awaitingTrust);
    }

    @Test public void replacedTrustAndCancelCannotChangeTheNewPrompt() {
        Fixture f = new Fixture();
        PinnedWebViewClient.SslPrompt old = prompt(f, f.first, true);
        f.current.set(f.second);
        PinnedWebViewClient.SslPrompt next = prompt(f, f.second, true);
        f.client.trustPrompt(old); f.client.returnFromPrompt(old, "old canceled");
        assertTrue(f.client.isAwaitingTrust()); assertTrue(f.effects.isEmpty());
        f.client.trustPrompt(next); f.client.trustPrompt(next);
        assertFalse(f.client.isAwaitingTrust());
        assertEquals(Collections.singletonList("onBindIdentity"), f.effects);
    }

    @Test public void reopenedPromptAtTheSameAddressStillRejectsOldListeners() {
        Fixture f = new Fixture();
        PinnedWebViewClient.SslPrompt old = prompt(f, f.first, true);
        PinnedWebViewClient.SslPrompt next = prompt(f, f.first, true);
        f.client.returnFromPrompt(old, "old canceled"); f.client.trustPrompt(old);
        assertTrue(f.client.isAwaitingTrust()); assertTrue(f.effects.isEmpty());
        f.client.returnFromPrompt(next, "canceled"); f.client.returnFromPrompt(next, "canceled");
        assertFalse(f.client.isAwaitingTrust());
        assertEquals(Collections.singletonList("onReturnToConnect"), f.effects);
    }

    @Test public void promptCannotActAfterItsPageIsReplacedEvenWithoutANewPrompt() {
        Fixture f = new Fixture();
        PinnedWebViewClient.SslPrompt old = prompt(f, f.first, true);
        f.current.set(f.second);
        f.client.trustPrompt(old); f.client.returnFromPrompt(old, "canceled");
        assertTrue(f.effects.isEmpty());
        f.client.clearPrompt(); assertFalse(f.client.isAwaitingTrust());
    }

    @Test public void changedCertificateNoticeCannotGrantTrust() {
        Fixture f = new Fixture();
        PinnedWebViewClient.SslPrompt changed = prompt(f, f.first, false);
        assertFalse(f.client.isAwaitingTrust());
        f.client.trustPrompt(changed); assertTrue(f.effects.isEmpty());
        f.client.returnFromPrompt(changed, "certificate changed");
        assertEquals(Collections.singletonList("onReturnToConnect"), f.effects);
    }

    @Test public void clearedPromptCannotGrantTrustOrNavigate() {
        Fixture f = new Fixture();
        PinnedWebViewClient.SslPrompt old = prompt(f, f.first, true);
        f.client.clearPrompt();
        f.client.trustPrompt(old); f.client.returnFromPrompt(old, "canceled");
        assertFalse(f.client.isAwaitingTrust()); assertTrue(f.effects.isEmpty());
    }
}
