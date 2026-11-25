package com.kuiwaiwai.plumelink;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CancellationException;
import java.util.concurrent.atomic.AtomicBoolean;
import org.junit.Test;
import static org.junit.Assert.*;

public class AudioCaptureSetupTest {
    @Test public void speechProcessingPrefersTheMicrophoneButRetainsRecognitionFallback() {
        List<String> attempts = new ArrayList<>();
        var opened = AudioCaptureSetup.open((rate, source) -> {
            attempts.add(source + ":" + rate);
            if (source == AudioCaptureSetup.Source.MICROPHONE) throw new IllegalArgumentException("unsupported");
            return new Device();
        }, () -> false, true);
        assertEquals(List.of("MICROPHONE:48000", "MICROPHONE:24000", "VOICE_RECOGNITION:48000"), attempts);
        assertEquals(AudioCaptureSetup.Source.VOICE_RECOGNITION, opened.source);
        opened.device.close();
    }
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
        assertEquals(6, error.getSuppressed().length);
    }

    @Test public void guaranteedRateOnlyRunsAfterAllExistingCandidatesAndClosesFailures() {
        List<String> attempts = new ArrayList<>();
        List<Device> devices = new ArrayList<>();
        var opened = AudioCaptureSetup.open((rate, source) -> {
            attempts.add(source + ":" + rate);
            Device device = new Device(); devices.add(device);
            device.initialized = rate == 44100 && source == AudioCaptureSetup.Source.MICROPHONE;
            return device;
        }, () -> false);
        assertEquals(List.of("VOICE_RECOGNITION:48000", "VOICE_RECOGNITION:24000", "MICROPHONE:48000",
            "MICROPHONE:24000", "VOICE_RECOGNITION:44100", "MICROPHONE:44100"), attempts);
        assertEquals(44100, opened.rate);
        assertTrue(devices.subList(0, 5).stream().allMatch(d -> d.closed));
        assertFalse(opened.device.closed);
        assertTrue(opened.device.started);
        opened.device.close();
    }

    @Test public void guaranteedRateRetainsSpeechProcessingSourcePreference() {
        List<String> attempts = new ArrayList<>();
        var opened = AudioCaptureSetup.open((rate, source) -> {
            attempts.add(source + ":" + rate);
            if (rate != 44100) throw new IllegalArgumentException("unsupported");
            return new Device();
        }, () -> false, true);
        assertEquals(List.of("MICROPHONE:48000", "MICROPHONE:24000", "VOICE_RECOGNITION:48000",
            "VOICE_RECOGNITION:24000", "MICROPHONE:44100"), attempts);
        assertEquals(44100, opened.rate);
        assertEquals(AudioCaptureSetup.Source.MICROPHONE, opened.source);
        opened.device.close();
    }

    @Test public void fallbackCancellationClosesTheCandidateWithoutTryingAnotherSource() {
        AtomicBoolean cancelled = new AtomicBoolean();
        Device fallback = new Device(); fallback.start = () -> cancelled.set(true);
        int[] attempts = {0};
        assertThrows(CancellationException.class, () -> AudioCaptureSetup.open((rate, source) -> {
            attempts[0]++;
            if (rate != 44100) throw new IllegalArgumentException("unsupported");
            return fallback;
        }, cancelled::get));
        assertEquals(5, attempts[0]);
        assertTrue(fallback.closed);
    }

    @Test public void fallbackPermissionFailureIsNotReclassifiedAsAnUnsupportedFormat() {
        Device fallback = new Device(); fallback.start = () -> { throw new SecurityException("revoked"); };
        int[] attempts = {0};
        assertThrows(SecurityException.class, () -> AudioCaptureSetup.open((rate, source) -> {
            attempts[0]++;
            if (rate != 44100) throw new IllegalArgumentException("unsupported");
            return fallback;
        }, () -> false));
        assertEquals(5, attempts[0]);
        assertTrue(fallback.closed);
    }
}
