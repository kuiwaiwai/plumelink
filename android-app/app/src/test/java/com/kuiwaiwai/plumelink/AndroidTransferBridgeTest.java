package com.kuiwaiwai.plumelink;

import android.webkit.WebView;
import java.lang.reflect.Proxy;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;
import static org.junit.Assert.*;

public class AndroidTransferBridgeTest {
    private static final class Fixture {
        final WebView first = new WebView(null), second = new WebView(null);
        final AtomicReference<WebView> current = new AtomicReference<>(first);
        final List<Runnable> pending = new ArrayList<>();
        final List<String> effects = new ArrayList<>();
        boolean switchDuringRead;
        final BridgeAccess<WebView> access = new BridgeAccess<>(current::get, pending::add);
        final AndroidTransferBridge.Host host = (AndroidTransferBridge.Host) Proxy.newProxyInstance(
            AndroidTransferBridge.Host.class.getClassLoader(), new Class<?>[]{AndroidTransferBridge.Host.class}, (proxy, method, args) -> {
                if (method.getName().equals("bridgeAccess")) return access;
                if (method.getName().equals("getStoredToken")) {
                    if (switchDuringRead) current.set(second);
                    return "b".repeat(64);
                }
                effects.add(method.getName());
                if (method.getReturnType() == boolean.class) return false;
                return method.getReturnType() == String.class ? "" : null;
            });
        final AndroidTransferBridge bridge = new AndroidTransferBridge(host, first);
        void flush() { List<Runnable> work = new ArrayList<>(pending); pending.clear(); work.forEach(Runnable::run); }
    }

    @Test public void oldPagesCannotChangeCredentialsOrControlNewTasks() {
        Fixture f = new Fixture(); f.current.set(f.second);
        f.bridge.setStoredToken(""); f.bridge.cancelFileTransfer(); f.bridge.startMicrophone(1);
        f.bridge.stopMicrophone(1); f.bridge.confirmSensitiveAction("shutdown");
        f.bridge.downloadFile("/api/files/1", "example.bin", "a".repeat(64));
        f.flush();
        assertTrue(f.effects.isEmpty());
        assertEquals("", f.bridge.getStoredToken());
    }
    @Test public void sharedFileConfirmationsStayWithTheirPageAndValidateBothIds() {
        Fixture f = new Fixture();
        f.bridge.sendSharedFiles("bad"); f.bridge.removeSharedFile("a".repeat(32), "bad");
        f.flush(); assertTrue(f.effects.isEmpty());
        f.bridge.sendSharedFiles("a".repeat(32)); f.current.set(f.second);
        f.flush(); assertTrue(f.effects.isEmpty());
        assertEquals("", f.bridge.getSharedFiles());
    }
    @Test public void metadataRetriesValidateIdsAndRemainBoundToTheirCurrentPage() {
        Fixture f = new Fixture();
        f.bridge.retrySharedFile("bad", "b".repeat(32));
        f.bridge.retrySharedFile("a".repeat(32), "bad");
        f.flush(); assertTrue(f.effects.isEmpty());
        f.bridge.retrySharedFile("a".repeat(32), "b".repeat(32));
        f.current.set(f.second); f.flush(); assertTrue(f.effects.isEmpty());
        f.current.set(f.first);
        f.bridge.retrySharedFile("a".repeat(32), "b".repeat(32)); f.flush();
        assertEquals(Collections.singletonList("retrySharedFile"), f.effects);
    }

    @Test public void connectionRecoveryValidatesDraftRequestAndRejectsCallsFromAnOldPage() {
        Fixture f = new Fixture(); String request = "a".repeat(32);
        f.bridge.recoverConnection("draft", "invalid"); f.bridge.recoverConnection("x".repeat(1001), request);
        f.bridge.recoverConnection("\0", request); f.bridge.cancelConnectionRecovery("invalid");
        f.flush(); assertTrue(f.effects.isEmpty());
        f.bridge.recoverConnection("中文\n\ttext", request); f.bridge.cancelConnectionRecovery(request);
        f.current.set(f.second); f.flush(); assertTrue(f.effects.isEmpty());
        assertEquals("", f.bridge.takeRecoveredDraft());
        f.current.set(f.first); f.bridge.recoverConnection("draft", request); f.bridge.cancelConnectionRecovery(request);
        f.flush(); assertEquals(List.of("recoverConnection", "cancelConnectionRecovery"), f.effects);
        f.bridge.takeRecoveredDraft(); assertEquals("takeRecoveredDraft", f.effects.get(2));
    }

    @Test public void queuedCallsCheckTheirSourceWhenExecuted() {
        Fixture f = new Fixture();
        f.bridge.setStoredToken("a".repeat(64));
        f.bridge.refreshReceiveFolder();
        f.bridge.setMobilePreferences(MobilePreferencesTest.sample());
        f.current.set(f.second); f.flush();
        assertTrue(f.effects.isEmpty());
    }

    @Test public void readsDiscardValuesIfThePageChangesDuringTheRead() {
        Fixture f = new Fixture(); f.switchDuringRead = true;
        assertEquals("", f.bridge.getStoredToken());
    }

    @Test public void currentPagesKeepValidActionsAndExistingValidation() {
        Fixture f = new Fixture();
        assertEquals("b".repeat(64), f.bridge.getStoredToken());
        f.bridge.setStoredToken("invalid"); f.bridge.setStoredDeviceId("bad");
        f.bridge.setStoredToken("a".repeat(64));
        f.bridge.downloadFile("/api/files/1", "example.bin", "a".repeat(64));
        f.bridge.cancelFileTransfer(); f.flush();
        assertEquals(Arrays.asList("setStoredToken", "startDownload", "cancelFileTransfer"), f.effects);
    }

    @Test public void folderRefreshUsesOnlyTheCurrentPage() {
        Fixture f = new Fixture();
        f.bridge.refreshReceiveFolder(); f.flush();
        assertEquals(Collections.singletonList("refreshReceiveFolder"), f.effects);
        f.effects.clear(); f.current.set(f.second);
        f.bridge.refreshReceiveFolder(); f.flush();
        assertTrue(f.effects.isEmpty());
    }
    @Test public void mobilePreferencesAcceptOnlyBoundedCurrentPageSnapshots() {
        Fixture f = new Fixture();
        f.bridge.setMobilePreferences("{}");
        f.bridge.setMobilePreferences("x".repeat(2049));
        f.bridge.setMobilePreferences(MobilePreferencesTest.sample());
        f.flush();
        assertEquals(Collections.singletonList("setMobilePreferences"), f.effects);
        f.effects.clear(); f.current.set(f.second);
        assertEquals("", f.bridge.getMobilePreferences());
        assertTrue(f.effects.isEmpty());
    }
}
