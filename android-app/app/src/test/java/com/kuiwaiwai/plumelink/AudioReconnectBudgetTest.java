package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.assertEquals;

public class AudioReconnectBudgetTest {
    @Test public void briefConfirmedStreamsCannotRenewTheRetryBudget() {
        AudioReconnectBudget budget = new AudioReconnectBudget();
        for (int expected : new int[]{1600, 3200, 0, 0}) {
            budget.beginAttempt(); budget.confirm();
            for (long now = 0; now <= 1000; now += 200) budget.live(now);
            assertEquals(expected, budget.retryDelayMillis());
        }
    }

    @Test public void sustainedConfirmedAudioAllowsRecoveryFromALaterOutage() {
        AudioReconnectBudget budget = new AudioReconnectBudget();
        assertEquals(1600, budget.retryDelayMillis());
        budget.confirm();
        for (long now = 10000; now <= 15000; now += 200) budget.live(now);
        assertEquals(1600, budget.retryDelayMillis());
        assertEquals(3200, budget.retryDelayMillis());
        assertEquals(0, budget.retryDelayMillis());
    }

    @Test public void deviceSleepOrAClockDiscontinuityCannotCountAsStableAudio() {
        for (long resumed : new long[]{900, 100000}) {
            AudioReconnectBudget budget = new AudioReconnectBudget();
            assertEquals(1600, budget.retryDelayMillis());
            budget.confirm(); budget.live(1000); budget.live(2000); budget.live(resumed);
            assertEquals(3200, budget.retryDelayMillis());
            assertEquals(0, budget.retryDelayMillis());
        }
    }

    @Test public void LocalCaptureWithoutReceiverConfirmationDoesNotRenewRetries() {
        AudioReconnectBudget budget = new AudioReconnectBudget();
        assertEquals(1600, budget.retryDelayMillis());
        for (long now = 0; now <= 10000; now += 200) budget.live(now);
        assertEquals(3200, budget.retryDelayMillis());
        budget.confirm();
        assertEquals(0, budget.retryDelayMillis());
    }

    @Test public void onlyAnExplicitNewSessionResetsAnExhaustedBudget() {
        AudioReconnectBudget budget = new AudioReconnectBudget();
        assertEquals(1600, budget.retryDelayMillis()); assertEquals(3200, budget.retryDelayMillis());
        for (int i = 0; i < 100; i++) assertEquals(0, budget.retryDelayMillis());
        budget.reset(); assertEquals(1600, budget.retryDelayMillis());
    }
}
