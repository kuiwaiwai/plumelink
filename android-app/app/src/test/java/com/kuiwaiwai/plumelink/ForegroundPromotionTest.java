package com.kuiwaiwai.plumelink;

import java.util.concurrent.atomic.AtomicInteger;
import org.junit.Test;
import static org.junit.Assert.*;

public class ForegroundPromotionTest {
    private static final int MIC = 128, FILES = 1, CONNECTION = 16;
    @Test public void rejectedNewMicrophonePreservesAnExistingTransfer() {
        AtomicInteger notifications = new AtomicInteger();
        int active = ForegroundPromotion.apply(FILES, FILES | MIC,
            types -> { throw new SecurityException("promotion denied"); }, notifications::incrementAndGet);
        assertFalse(ForegroundPromotion.covers(active, MIC));
        assertTrue(ForegroundPromotion.covers(active, FILES));
        assertEquals(FILES, active); assertEquals(1, notifications.get());
    }
    @Test public void rejectedNewTransferPreservesContinuousMicrophone() {
        int active = ForegroundPromotion.apply(MIC, MIC | FILES,
            types -> { throw new IllegalStateException("background start denied"); }, () -> {});
        assertTrue(ForegroundPromotion.covers(active, MIC));
        assertFalse(ForegroundPromotion.covers(active, FILES));
    }
    @Test public void anInitialFailureCannotClaimForegroundProtection() {
        int active = ForegroundPromotion.apply(0, CONNECTION,
            types -> { throw new SecurityException(); }, () -> { throw new SecurityException(); });
        assertEquals(0, active); assertFalse(ForegroundPromotion.covers(active, CONNECTION));
    }
    @Test public void notificationFailureDoesNotRemoveAlreadyAcceptedProtection() {
        int active = ForegroundPromotion.apply(MIC, MIC,
            types -> fail("already registered"), () -> { throw new SecurityException(); });
        assertTrue(ForegroundPromotion.covers(active, MIC));
    }
    @Test public void aRejectedDowngradeStillCoversItsRemainingTask() {
        int active = ForegroundPromotion.apply(MIC | FILES, MIC,
            types -> { throw new IllegalStateException(); }, () -> {});
        assertTrue(ForegroundPromotion.covers(active, MIC));
        assertEquals(MIC | FILES, active);
    }
    @Test public void successfulPromotionRecordsOnlyTheRequestedTypes() {
        AtomicInteger registered = new AtomicInteger();
        assertEquals(MIC | FILES, ForegroundPromotion.apply(FILES, MIC | FILES, registered::set,
            () -> fail("promotion already updates the notification")));
        assertEquals(MIC | FILES, registered.get());
    }
    @Test public void aStoppedMicrophoneCannotLeaveUnsupportedKeepAliveBehind() {
        int active = ForegroundPromotion.apply(MIC, CONNECTION,
            types -> { throw new SecurityException("type change denied"); }, () -> {});
        assertEquals(MIC, active);
        assertTrue(ForegroundPromotion.shouldRetireIdle(false, active, CONNECTION));
    }
    @Test public void idleRetirementNeverStopsAnotherActiveTask() {
        assertFalse(ForegroundPromotion.shouldRetireIdle(true, MIC, CONNECTION));
        assertFalse(ForegroundPromotion.shouldRetireIdle(true, FILES, CONNECTION));
        assertFalse(ForegroundPromotion.shouldRetireIdle(false, CONNECTION, CONNECTION));
        assertTrue(ForegroundPromotion.shouldRetireIdle(false, 0, CONNECTION));
    }
}
