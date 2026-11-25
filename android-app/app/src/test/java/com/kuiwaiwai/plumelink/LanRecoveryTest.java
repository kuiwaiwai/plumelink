package com.kuiwaiwai.plumelink;

import java.util.*;
import java.util.function.Consumer;
import org.junit.Test;
import static org.junit.Assert.*;

public class LanRecoveryTest {
    private static final String ID = "a".repeat(24), PIN = "saved-pin", OLD = "https://192.168.1.2:17840";
    private static final class Host implements LanRecovery.Host {
        boolean current = true, throwDiscovery;
        Runnable changed, timeout;
        final Map<String,String> nearby = new LinkedHashMap<>();
        final List<String> probes = new ArrayList<>(), phases = new ArrayList<>(), recovered = new ArrayList<>();
        final List<Consumer<LanRecovery.Result>> replies = new ArrayList<>();
        int stopped, cancelled, cleared;
        public boolean current() { return current; }
        public void discover(Runnable update) { changed = update; if (throwDiscovery) throw new IllegalStateException("unavailable"); }
        public Map<String,String> discovered() { return nearby; }
        public void probe(String url, Consumer<LanRecovery.Result> result) { probes.add(url); replies.add(result); }
        public void stopDiscovery() { stopped++; }
        public void cancelProbe() { cancelled++; }
        public void deadline(Runnable action) { timeout = action; }
        public void clearDeadline() { cleared++; }
        public void phase(String value) { phases.add(value); }
        public void recovered(String url, String draft) { recovered.add(url + "\n" + draft); }
    }
    private LanRecovery start(Host host) {
        LanRecovery recovery = new LanRecovery(host, ID, PIN, OLD, "draft\n\t中文", "b".repeat(32));
        recovery.start(); return recovery;
    }
    @Test public void sameAddressCanRecoverOnlyWithTheOriginalIdentityAndCertificate() {
        Host host = new Host(); start(host);
        assertEquals(List.of(OLD), host.probes);
        host.replies.get(0).accept(new LanRecovery.Result(true, ID, PIN));
        assertEquals(List.of(OLD + "\ndraft\n\t中文"), host.recovered);
        assertEquals(1, host.stopped); assertEquals(1, host.cancelled); assertEquals(1, host.cleared);
        host.timeout.run(); assertEquals(List.of("searching"), host.phases);
    }
    @Test public void discoveryIsOnlyAHintAndWrongPinsOrIdentitiesCannotCommit() {
        Host host = new Host(); start(host);
        host.nearby.put("other-device", "c".repeat(24));
        host.nearby.put("forged-address", ID); host.nearby.put("changed-address", ID); host.nearby.put("correct-address", ID);
        host.changed.run();
        host.replies.get(0).accept(new LanRecovery.Result(false, "", ""));
        host.replies.get(1).accept(new LanRecovery.Result(true, ID, "new-pin"));
        host.replies.get(2).accept(new LanRecovery.Result(true, "c".repeat(24), PIN));
        assertTrue(host.recovered.isEmpty()); assertFalse(host.probes.contains("other-device"));
        host.replies.get(3).accept(new LanRecovery.Result(true, ID, PIN));
        assertEquals(List.of("correct-address\ndraft\n\t中文"), host.recovered);
    }
    @Test public void timeoutCancelsRealWorkAndRejectsALateSuccessfulProbe() {
        Host host = new Host(); start(host); host.timeout.run();
        host.replies.get(0).accept(new LanRecovery.Result(true, ID, PIN));
        host.nearby.put("new", ID); host.changed.run();
        assertEquals(List.of("searching", "error"), host.phases); assertTrue(host.recovered.isEmpty());
        assertEquals(1, host.probes.size()); assertEquals(1, host.stopped); assertEquals(1, host.cancelled);
    }
    @Test public void explicitCancellationIsIdempotentAndCannotBeUndoneByOldCallbacks() {
        Host host = new Host(); LanRecovery recovery = start(host); recovery.cancel(); recovery.cancel();
        host.replies.get(0).accept(new LanRecovery.Result(true, ID, PIN)); host.timeout.run();
        assertEquals(List.of("searching", "cancelled"), host.phases); assertTrue(host.recovered.isEmpty());
        assertEquals("b".repeat(32), recovery.request()); assertEquals(1, host.stopped);
    }
    @Test public void changedPageNetworkForegroundOrConnectionOwnershipBlocksPublication() {
        Host host = new Host(); start(host); host.current = false;
        host.replies.get(0).accept(new LanRecovery.Result(true, ID, PIN));
        assertTrue(host.recovered.isEmpty()); assertEquals(List.of("searching", "cancelled"), host.phases);
    }
    @Test public void discoveryAndDelayedRepliesCannotDuplicateAProbeOrExceedTheRoundBudget() {
        Host host = new Host(); start(host);
        for (int i = 0; i < 20; i++) host.nearby.put("address-" + i, ID);
        host.changed.run(); host.changed.run();
        Consumer<LanRecovery.Result> old = host.replies.get(0);
        old.accept(new LanRecovery.Result(false, "", ""));
        old.accept(new LanRecovery.Result(true, ID, PIN)); assertTrue(host.recovered.isEmpty());
        for (int i = 1; i < 8; i++) host.replies.get(i).accept(new LanRecovery.Result(false, "", ""));
        assertEquals(8, host.probes.size()); host.changed.run(); assertEquals(8, host.probes.size());
        host.timeout.run(); assertEquals(List.of("searching", "error"), host.phases);
    }
    @Test public void discoveryStartupFailureReleasesAllResourcesAndKeepsTheRoundRetryable() {
        Host host = new Host(); host.throwDiscovery = true; start(host);
        assertEquals(List.of("searching", "error"), host.phases);
        assertEquals(1, host.stopped); assertEquals(1, host.cancelled); assertEquals(1, host.cleared);
        assertTrue(host.recovered.isEmpty());
    }
    @Test public void draftsAreBoundedAndAllowOnlyTextNewlinesAndTabs() {
        assertTrue(LanRecovery.validDraft("中文\n\r\t😀")); assertTrue(LanRecovery.validDraft("a".repeat(1000)));
        for (String value : new String[]{null, "a".repeat(1001), "\0", "\b", "\u007f"}) assertFalse(LanRecovery.validDraft(value));
    }
    @Test public void recoveredDraftIsRamOnlySingleUseAndBoundToTheNewPageIdentityAndOwner() {
        LanRecovery.Draft<Object> draft = new LanRecovery.Draft<>(); Object old = new Object(), current = new Object();
        draft.set(current, ID, "owner", "private text");
        assertEquals("", draft.take(old, ID, "owner")); assertEquals("", draft.take(current, "other", "owner"));
        assertEquals("", draft.take(current, ID, "other")); assertEquals("private text", draft.take(current, ID, "owner"));
        assertEquals("", draft.take(current, ID, "owner")); draft.set(current, ID, "owner", "text"); draft.clear();
        assertEquals("", draft.take(current, ID, "owner"));
    }
}
