package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;

public class AudioCaptureProgressTest {
    @Test public void slowFormatFallbackCanFinishBeforeTheFirstReadDeadlineStarts() {
        AudioCaptureProgress progress = new AudioCaptureProgress();
        progress.prepare(0);
        long[] now = {0};
        int[] attempts = {0}, closed = {0};
        var opened = AudioCaptureSetup.open((rate, source) -> {
            now[0] += 600;
            int attempt = ++attempts[0];
            assertEquals(AudioCaptureProgress.Timeout.NONE, progress.timeout(now[0]));
            return new AudioCaptureSetup.Device() {
                public boolean initialized() { return attempt == 3; }
                public void start() {}
                public boolean recording() { return true; }
                public void close() { closed[0]++; }
            };
        }, () -> false);
        assertEquals(3, attempts[0]);
        assertEquals(2, closed[0]);
        assertTrue(progress.beginReading(now[0]));
        assertTrue(progress.progress(now[0] + 20));
        assertEquals(AudioCaptureProgress.Timeout.NONE, progress.timeout(3019));
        assertEquals(AudioCaptureProgress.Timeout.DATA, progress.timeout(3020));
        opened.device.close();
    }

    @Test public void repeatedPreparationCannotExtendItsDeadline() {
        AudioCaptureProgress progress = new AudioCaptureProgress();
        progress.prepare(0); progress.prepare(2500);
        assertEquals(AudioCaptureProgress.Timeout.NONE, progress.timeout(2999));
        assertEquals(AudioCaptureProgress.Timeout.PREPARATION, progress.timeout(3000));
        assertFalse(progress.beginReading(3000));
    }

    @Test public void aBlockedFirstReadKeepsTheShortDataDeadline() {
        AudioCaptureProgress progress = new AudioCaptureProgress();
        progress.prepare(0); assertTrue(progress.beginReading(2400));
        assertEquals(AudioCaptureProgress.Timeout.NONE, progress.timeout(3599));
        assertEquals(AudioCaptureProgress.Timeout.DATA, progress.timeout(3600));
        assertFalse(progress.progress(3600));
        assertEquals(AudioCaptureProgress.Timeout.DATA, progress.timeout(3601));
    }

    @Test public void lateDataCannotHideAnExpiredReadWhileTheUiThreadIsBusy() {
        AudioCaptureProgress progress = new AudioCaptureProgress();
        progress.prepare(0); assertTrue(progress.beginReading(100));
        assertTrue(progress.progress(120));
        assertEquals(AudioCaptureProgress.Timeout.NONE, progress.timeout(1319));
        assertFalse(progress.progress(1320));
        assertEquals(AudioCaptureProgress.Timeout.DATA, progress.timeout(1320));
    }

    @Test public void oldCallbacksCannotReviveAnUnstartedOrFinishedCapture() {
        AudioCaptureProgress progress = new AudioCaptureProgress();
        assertFalse(progress.beginReading(0)); assertFalse(progress.progress(0));
        assertEquals(AudioCaptureProgress.Timeout.NONE, progress.timeout(100_000));
        progress.finish(); progress.prepare(0);
        assertFalse(progress.beginReading(0)); assertFalse(progress.progress(100));
        assertEquals(AudioCaptureProgress.Timeout.NONE, progress.timeout(100_000));
    }

    @Test public void aDuplicateReadStartCannotRenewTheReadDeadline() {
        AudioCaptureProgress progress = new AudioCaptureProgress();
        progress.prepare(0); assertTrue(progress.beginReading(300));
        assertFalse(progress.beginReading(900));
        assertEquals(AudioCaptureProgress.Timeout.DATA, progress.timeout(1500));
    }
}
