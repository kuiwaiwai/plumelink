package com.kuiwaiwai.plumelink;

import java.util.*;
import org.junit.Test;
import static org.junit.Assert.*;

public class AudioCaptureEffectsTest {
    private static final class Effect implements AudioCaptureEffects.Effect {
        boolean enabled = true;
        int closed;
        RuntimeException failure;
        public boolean enable() { return enabled; }
        public void close() { closed++; if (failure != null) throw failure; }
    }
    @Test public void standardCaptureDoesNotRequestAutomaticGain() {
        List<AudioCaptureEffects.Kind> requests = new ArrayList<>();
        Effect noise = new Effect();
        try (AudioCaptureEffects effects = new AudioCaptureEffects(true, false, kind -> { requests.add(kind); return noise; })) {
            assertTrue(effects.noiseSuppression); assertFalse(effects.automaticGain);
        }
        assertEquals(Collections.singletonList(AudioCaptureEffects.Kind.NOISE_SUPPRESSION), requests);
        assertEquals(1, noise.closed);
    }
    @Test public void unsupportedOrRejectedProcessingDoesNotBlockOtherEffectsOrCapture() {
        Effect rejected = new Effect(); rejected.enabled = false;
        AudioCaptureEffects effects = new AudioCaptureEffects(true, true, kind -> kind == AudioCaptureEffects.Kind.NOISE_SUPPRESSION ? null : rejected);
        assertFalse(effects.noiseSuppression); assertFalse(effects.automaticGain); assertEquals(1, rejected.closed);
        effects.close(); assertEquals(1, rejected.closed);
        Effect gain = new Effect();
        try (AudioCaptureEffects partial = new AudioCaptureEffects(true, true, kind -> {
            if (kind == AudioCaptureEffects.Kind.NOISE_SUPPRESSION) throw new UnsupportedOperationException();
            return gain;
        })) { assertFalse(partial.noiseSuppression); assertTrue(partial.automaticGain); }
        assertEquals(1, gain.closed);
    }
    @Test public void teardownReleasesEveryOwnedEffectEvenWhenOneFails() {
        Effect noise = new Effect(), gain = new Effect(); noise.failure = new IllegalStateException("release");
        AudioCaptureEffects effects = new AudioCaptureEffects(true, true, kind -> kind == AudioCaptureEffects.Kind.NOISE_SUPPRESSION ? noise : gain);
        assertThrows(IllegalStateException.class, effects::close);
        effects.close(); assertEquals(1, noise.closed); assertEquals(1, gain.closed);
    }
}
