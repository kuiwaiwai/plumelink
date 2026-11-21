package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;

public class AudioSendWindowTest {
    @Test public void shortStallKeepsWindowBoundedAndRecoversWithoutReplaying() {
        AudioSendWindow window = new AudioSendWindow();
        for (int i = 0; i < 4; i++) assertEquals(i + 1, window.reserve(i * 20, 0));
        for (int now = 80; now <= 1000; now += 20) {
            assertEquals(0, window.reserve(now, 0));
            assertFalse(window.expired(now));
        }
        assertTrue(window.acknowledge(4));
        assertEquals(5, window.reserve(1020, 0));
        assertFalse(window.expired(1200));
        assertTrue(window.expired(2220));
    }

    @Test public void partialAckDoesNotRenewOldOutstandingFrames() {
        AudioSendWindow window = new AudioSendWindow();
        for (int i = 0; i < 4; i++) window.reserve(i * 20, 0);
        assertTrue(window.acknowledge(1));
        assertEquals(5, window.reserve(1100, 0));
        assertFalse(window.expired(1219));
        assertTrue(window.expired(1220));
        assertTrue(window.acknowledge(4));
        assertFalse(window.expired(1220));
        assertTrue(window.expired(2300));
    }

    @Test public void idleCaptureWithoutPendingAudioIsNotAnAckFailure() {
        AudioSendWindow window = new AudioSendWindow();
        assertFalse(window.expired(10_000));
        assertEquals(1, window.reserve(10_000, 0));
        assertTrue(window.acknowledge(1));
        assertFalse(window.expired(20_000));
    }

    @Test public void invalidAcksCannotReleaseOrExtendWindow() {
        AudioSendWindow window = new AudioSendWindow();
        window.reserve(0, 0);
        assertFalse(window.acknowledge(-1));
        assertFalse(window.acknowledge(0));
        assertFalse(window.acknowledge(2));
        assertTrue(window.expired(1200));
        assertTrue(window.acknowledge(1));
        assertFalse(window.acknowledge(1));
    }

    @Test public void transportBacklogBlocksNewFramesAndRingCanBeReused() {
        AudioSendWindow window = new AudioSendWindow();
        assertEquals(0, window.reserve(0, 972 * 4));
        for (int i = 1; i <= 100; i++) {
            assertEquals(i, window.reserve(i * 20, 0));
            assertFalse(window.expired(i * 20 + 1199));
            assertTrue(window.acknowledge(i));
        }
    }
}
