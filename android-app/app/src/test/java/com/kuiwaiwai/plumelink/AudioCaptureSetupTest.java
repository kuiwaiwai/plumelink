package com.kuiwaiwai.plumelink;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CancellationException;
import java.util.concurrent.atomic.AtomicBoolean;
import org.junit.Test;
import static org.junit.Assert.*;

public class AudioCaptureSetupTest {
    private static final class Device implements AudioCaptureSetup.Device {
        boolean initialized = true, recording = true, closed, started;
        Runnable start = () -> {};
        public boolean initialized() { return initialized; }
        public void start() { started = true; start.run(); }
        public boolean recording() { return recording; }
        public void close() { assertFalse(closed); closed = true; }
    }

    @Test public void fallsBackAcrossFormatsAndSourcesAndClosesEveryFailedCandidate() {
        List<String> attempts = new ArrayList<>();
        List<Device> devices = new ArrayList<>();
        var opened = AudioCaptureSetup.open((rate, source) -> {
            attempts.add(source + ":" + rate);
            Device device = new Device(); devices.add(device);
            if (devices.size() == 1) device.initialized = false;
            if (devices.size() == 2) device.start = () -> { throw new IllegalStateException("start failed"); };
            if (devices.size() == 3) device.recording = false;
            return device;
        }, () -> false);
        assertEquals(List.of("VOICE_RECOGNITION:48000", "VOICE_RECOGNITION:24000", "MICROPHONE:48000", "MICROPHONE:24000"), attempts);
        assertEquals(24000, opened.rate);
        assertEquals(AudioCaptureSetup.Source.MICROPHONE, opened.source);
        assertTrue(devices.subList(0, 3).stream().allMatch(d -> d.closed));
        assertFalse(opened.device.closed);
    }

    @Test public void permissionFailureNeverRetriesAnotherSource() {
        Device device = new Device();
        device.start = () -> { throw new SecurityException("revoked"); };
        int[] attempts = {0};
        assertThrows(SecurityException.class, () -> AudioCaptureSetup.open((rate, source) -> {
            attempts[0]++; return device;
        }, () -> false));
        assertEquals(1, attempts[0]);
        assertTrue(device.closed);
    }

    @Test public void cancellationDuringCreationClosesWithoutStartingOrRetrying() {
        AtomicBoolean cancelled = new AtomicBoolean();
        Device device = new Device();
        assertThrows(CancellationException.class, () -> AudioCaptureSetup.open((rate, source) -> {
            cancelled.set(true); return device;
        }, cancelled::get));
        assertFalse(device.started);
        assertTrue(device.closed);
    }

    @Test public void cancellationDuringStartClosesTheRecordingDevice() {
        AtomicBoolean cancelled = new AtomicBoolean();
        Device device = new Device(); device.start = () -> cancelled.set(true);
        assertThrows(CancellationException.class, () -> AudioCaptureSetup.open((rate, source) -> device, cancelled::get));
        assertTrue(device.closed);
    }

    @Test public void unsupportedFormatsProduceBoundedAttemptsAndUsefulCauses() {
        IllegalStateException error = assertThrows(IllegalStateException.class,
            () -> AudioCaptureSetup.open((rate, source) -> { throw new IllegalArgumentException("unsupported " + rate); }, () -> false));
        assertEquals(4, error.getSuppressed().length);
    }
}
