package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;

public class MainActivityTest {
    private static void set(Object target, String name, Object value) throws Exception {
        java.lang.reflect.Field field = target.getClass().getDeclaredField(name);
        field.setAccessible(true); field.set(target, value);
    }
    private static Object get(Object target, String name) throws Exception {
        java.lang.reflect.Field field = target.getClass().getDeclaredField(name);
        field.setAccessible(true); return field.get(target);
    }
    private static final class RecordingView extends android.webkit.WebView {
        final java.util.List<String> scripts = new java.util.ArrayList<>();
        RecordingView() { super(null); }
        @Override public void evaluateJavascript(String script, android.webkit.ValueCallback<String> result) { scripts.add(script); }
    }
    private static void emitSharedFiles(MainActivity activity, boolean open) throws Exception {
        java.lang.reflect.Method method = MainActivity.class.getDeclaredMethod("emitSharedFiles", boolean.class);
        method.setAccessible(true); method.invoke(activity, open);
    }
    private static ShareInbox inbox(MainActivity activity) throws Exception {
        java.lang.reflect.Field field = MainActivity.class.getDeclaredField("shareInbox");
        field.setAccessible(true); return (ShareInbox)field.get(activity);
    }

    @Test public void cancellingARecoveryCannotAffectAnotherRequestOrAnotherPage() throws Exception {
        MainActivity activity = new MainActivity(); RecordingView previous = new RecordingView(), current = new RecordingView();
        java.util.List<String> phases = new java.util.ArrayList<>();
        LanRecovery.Host host = (LanRecovery.Host)java.lang.reflect.Proxy.newProxyInstance(LanRecovery.Host.class.getClassLoader(),
            new Class<?>[]{LanRecovery.Host.class}, (proxy, method, args) -> {
                if (method.getName().equals("current")) return true;
                if (method.getName().equals("discovered")) return java.util.Map.of();
                if (method.getName().equals("phase")) phases.add((String)args[0]);
                return null;
            });
        LanRecovery recovery = new LanRecovery(host, "a".repeat(24), "pin", "old", "draft", "b".repeat(32));
        recovery.start(); set(activity, "webView", current); set(activity, "connectionRecovery", recovery);
        activity.cancelConnectionRecovery(current, "c".repeat(32));
        activity.cancelConnectionRecovery(previous, "b".repeat(32));
        assertEquals(java.util.List.of("searching"), phases);
        activity.cancelConnectionRecovery(current, "b".repeat(32));
        assertEquals(java.util.List.of("searching", "cancelled"), phases);
        LanRecovery next = new LanRecovery(host, "a".repeat(24), "pin", "new", "draft", "c".repeat(32));
        next.start(); set(activity, "connectionRecovery", next);
        activity.cancelConnectionRecovery(current, "b".repeat(32));
        assertEquals(java.util.List.of("searching", "cancelled", "searching"), phases);
        activity.cancelConnectionRecovery(current, "c".repeat(32));
    }

    @Test public void connectionRecoveryRecognizesOwnedCaptureAndOutstandingFilePickersAsBusy() throws Exception {
        MainActivity activity = new MainActivity();
        java.lang.reflect.Method method = MainActivity.class.getDeclaredMethod("recoveryBusy"); method.setAccessible(true);
        assertEquals(false, method.invoke(activity));
        set(activity, "uploadTarget", new android.os.Bundle()); assertEquals(true, method.invoke(activity));
        set(activity, "uploadTarget", null);
        RemoteSessionService service = new RemoteSessionService();
        set(service, "base", "https://192.168.1.2:17840"); set(service, "micActive", true); set(activity, "session", service);
        assertEquals(true, method.invoke(activity));
        set(service, "micActive", false); assertEquals(false, method.invoke(activity));
        set(service, "transferring", true); assertEquals("", service.activeBase());
        assertEquals(true, method.invoke(activity));
    }

    @Test public void oldPageRecoveryDoesNotInterruptANewPcProbeOrItsPendingTrustConfirmation() throws Exception {
        for (int mode = 0; mode < 3; mode++) {
            MainActivity activity = new MainActivity(); RecordingView current = new RecordingView();
            String old = "https://192.168.1.2:17840", next = "https://192.168.1.3:17840", request = "d".repeat(32);
            set(activity, "webView", current); set(activity, "activityResumed", true);
            set(activity, "currentBaseUrl", old); set(activity, "currentComputerIdentity", "a".repeat(24));
            set(activity, "connectionStatus", "offline"); set(activity, "connectionGeneration", 7);
            set(activity, "connectionRecoveryText", (java.util.function.IntFunction<String>)id -> {
                assertEquals(R.string.ui_connection_recovery_unavailable, id); return "Recovery unavailable";
            });
            ConnectionProbe probe = new ConnectionProbe();
            android.app.AlertDialog trust = new android.app.AlertDialog(null) {};
            if (mode == 0) set(activity, "probingUrl", next);
            if (mode == 1) set(activity, "connectionProbe", probe);
            if (mode == 2) set(activity, "connectionTrustDialog", trust);
            activity.recoverConnection(current, "draft", request);
            assertEquals(mode == 0 ? next : "", get(activity, "probingUrl"));
            assertSame(mode == 1 ? probe : null, get(activity, "connectionProbe"));
            assertSame(mode == 2 ? trust : null, get(activity, "connectionTrustDialog"));
            assertEquals(false, get(probe, "cancelled"));
            assertEquals(old, get(activity, "currentBaseUrl")); assertEquals("a".repeat(24), get(activity, "currentComputerIdentity"));
            assertEquals(7, get(activity, "connectionGeneration")); assertSame(current, get(activity, "webView"));
            assertNull(get(activity, "connectionRecovery")); assertNull(get(activity, "recoveryDiscovery")); assertNull(get(activity, "recoveryProbe"));
            assertEquals(1, current.scripts.size());
            assertTrue(current.scripts.get(0).contains("\"request\":\"" + request + "\""));
            assertTrue(current.scripts.get(0).contains("\"phase\":\"error\""));
            assertTrue(current.scripts.get(0).contains("Recovery unavailable"));
        }
    }

    @Test public void backgroundShareOpensOnceAfterForegroundWithoutMetadataReopeningIt() throws Exception {
        MainActivity activity = new MainActivity(); RecordingView current = new RecordingView();
        set(activity, "webView", current);
        emitSharedFiles(activity, true);
        emitSharedFiles(activity, false);
        assertTrue(current.scripts.isEmpty());
        set(activity, "activityResumed", true);
        emitSharedFiles(activity, false);
        assertEquals(1, current.scripts.size());
        assertTrue(current.scripts.get(0).contains("\"open\":true"));
        emitSharedFiles(activity, false);
        assertEquals(2, current.scripts.size());
        assertTrue(current.scripts.get(1).contains("\"open\":false"));
    }

    @Test public void shareWithoutAPageRetainsOpenIntentUntilTheCurrentForegroundPage() throws Exception {
        MainActivity activity = new MainActivity();
        emitSharedFiles(activity, true);
        RecordingView previous = new RecordingView(), current = new RecordingView();
        set(activity, "webView", previous);
        emitSharedFiles(activity, false);
        set(activity, "webView", current); set(activity, "activityResumed", true);
        emitSharedFiles(activity, false);
        assertTrue(previous.scripts.isEmpty());
        assertEquals(1, current.scripts.size());
        assertTrue(current.scripts.get(0).contains("\"open\":true"));
    }

    @Test public void clearingOrRemovingTheLastSharedFileCancelsDeferredNavigation() throws Exception {
        for (boolean remove : new boolean[]{false, true}) {
            MainActivity activity = new MainActivity(); ShareInbox inbox = inbox(activity);
            inbox.add(java.util.List.of("content://example.provider/file"));
            emitSharedFiles(activity, true);
            if (remove) activity.removeSharedFile(inbox.revision(), inbox.files().get(0).id());
            else activity.clearSharedFiles(inbox.revision());
            RecordingView current = new RecordingView();
            set(activity, "webView", current); set(activity, "activityResumed", true);
            emitSharedFiles(activity, false);
            assertEquals(1, current.scripts.size());
            assertTrue(current.scripts.get(0).contains("\"open\":false"));
        }
    }
    @Test public void microphonePermissionResultsWithoutAnOwnedRequestDoNotResetThePage() throws Exception {
        MainActivity activity = new MainActivity();
        RecordingView current = new RecordingView(); set(activity, "webView", current);
        activity.onRequestPermissionsResult(8130, new String[]{android.Manifest.permission.RECORD_AUDIO}, new int[]{android.content.pm.PackageManager.PERMISSION_GRANTED});
        assertTrue(current.scripts.isEmpty());
    }
    @Test public void oldMicrophonePermissionResultsDoNotReachANewPage() throws Exception {
        MainActivity activity = new MainActivity();
        RecordingView previous = new RecordingView(), current = new RecordingView();
        set(activity, "webView", previous); activity.requestMicrophonePermission(previous, 7);
        set(activity, "webView", current); activity.requestMicrophonePermission(current, 8);
        current.scripts.clear();
        activity.onRequestPermissionsResult(8130, new String[]{android.Manifest.permission.RECORD_AUDIO}, new int[]{android.content.pm.PackageManager.PERMISSION_GRANTED});
        assertTrue(current.scripts.isEmpty());
        activity.requestMicrophonePermission(current, 9);
        assertSame(current, activity.takeMicrophonePermissionSource());
        assertNull(activity.takeMicrophonePermissionSource());
    }
    @Test public void permissionRequestsKeepTheirOwnerUntilTheSystemResult() throws Exception {
        MainActivity activity = new MainActivity(); RecordingView current = new RecordingView();
        set(activity, "webView", current);
        activity.requestMicrophonePermission(current, 7); activity.requestMicrophonePermission(current, 8);
        java.lang.reflect.Field request = MainActivity.class.getDeclaredField("pendingMicrophoneRequest"); request.setAccessible(true);
        assertEquals(7, request.getInt(activity));
        assertSame(current, activity.takeMicrophonePermissionSource());
        assertEquals(0, request.getInt(activity));
    }
    @Test public void permissionRetryFromAnOldPageDoesNotReserveTheNewPage() throws Exception {
        MainActivity activity = new MainActivity(); RecordingView previous = new RecordingView(), current = new RecordingView();
        set(activity, "webView", current);
        activity.requestMicrophonePermission(previous, 4);
        assertTrue(current.scripts.isEmpty());
        activity.requestMicrophonePermission(current, 5);
        assertSame(current, activity.takeMicrophonePermissionSource());
    }
    @Test public void otherPermissionResultsDoNotConsumeTheMicrophoneRequest() throws Exception {
        MainActivity activity = new MainActivity(); RecordingView current = new RecordingView();
        set(activity, "webView", current); activity.requestMicrophonePermission(current, 7);
        activity.onRequestPermissionsResult(8131, new String[]{android.Manifest.permission.POST_NOTIFICATIONS}, new int[]{android.content.pm.PackageManager.PERMISSION_GRANTED});
        assertSame(current, activity.takeMicrophonePermissionSource());
    }
    @Test public void completedFileTasksStayWithTheirComputer() {
        String first = "https://192.0.2.1:17840", second = "https://192.0.2.2:17840";
        assertFalse(MainActivity.acceptsSessionEvent("plumelink-native-transfer", first, second, ""));
        assertFalse(MainActivity.acceptsSessionEvent("plumelink-native-transfer", first, second, second));
        assertTrue(MainActivity.acceptsSessionEvent("plumelink-native-transfer", first, first, ""));
        assertFalse(MainActivity.acceptsSessionEvent("plumelink-native-transfer", "", second, ""));
    }

    @Test public void localDownloadRecoveryRemainsAvailableAfterSwitchingComputers() {
        assertTrue(MainActivity.acceptsSessionEvent("plumelink-download-recovery", "", "https://192.0.2.2:17840", "https://192.0.2.1:17840"));
        assertFalse(MainActivity.acceptsSessionEvent("plumelink-native-transfer", "https://192.0.2.1:17840", "https://192.0.2.2:17840", ""));
    }

    @Test public void rejectedRequestsReachTheirComputerWhileAnotherTaskStops() {
        String first = "https://192.0.2.1:17840", second = "https://192.0.2.2:17840";
        assertTrue(MainActivity.acceptsSessionEvent("plumelink-native-transfer", second, second, first));
        assertFalse(MainActivity.acceptsSessionEvent("plumelink-microphone", "", second, first));
        assertTrue(MainActivity.acceptsSessionEvent("plumelink-microphone", "", second, ""));
    }

    @Test public void pairingCodesAreConsumedOnlyByTheirCurrentPage() throws Exception {
        MainActivity activity = new MainActivity();
        RecordingView previous = new RecordingView(), current = new RecordingView();
        set(activity, "webView", current); set(activity, "currentBaseUrl", "https://192.0.2.2:17840");
        set(activity, "pendingPairUrl", "https://192.0.2.2:17840"); set(activity, "pendingPairCode", "123456");
        assertEquals("", activity.takePairingCode(previous));
        assertEquals("123456", activity.takePairingCode(current));
        assertEquals("", activity.takePairingCode(current));
    }

    @Test public void deviceConfirmationCannotMoveToAnotherPage() throws Exception {
        MainActivity activity = new MainActivity();
        RecordingView previous = new RecordingView(), current = new RecordingView();
        set(activity, "webView", current); set(activity, "pendingSensitiveAction", "shutdown");
        set(activity, "pendingSensitiveSource", new java.lang.ref.WeakReference<>(previous));
        activity.onActivityResult(8128, android.app.Activity.RESULT_OK, null);
        assertTrue(current.scripts.isEmpty());
        set(activity, "pendingSensitiveAction", "shutdown");
        set(activity, "pendingSensitiveSource", new java.lang.ref.WeakReference<>(current));
        activity.onActivityResult(8128, android.app.Activity.RESULT_CANCELED, null);
        assertEquals(1, current.scripts.size());
        assertTrue(current.scripts.get(0).contains("approved:false"));
        set(activity, "pendingSensitiveAction", "shutdown");
        set(activity, "pendingSensitiveSource", new java.lang.ref.WeakReference<>(current));
        activity.onActivityResult(8128, android.app.Activity.RESULT_OK, null);
        assertEquals(2, current.scripts.size());
        assertTrue(current.scripts.get(1).contains("approved:true"));
    }
}
