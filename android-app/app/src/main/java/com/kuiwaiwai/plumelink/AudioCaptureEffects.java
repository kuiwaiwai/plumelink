package com.kuiwaiwai.plumelink;

import java.util.*;

final class AudioCaptureEffects implements AutoCloseable {
    enum Kind { NOISE_SUPPRESSION, AUTOMATIC_GAIN }
    interface Effect extends AutoCloseable {
        boolean enable();
        @Override void close();
    }
    interface Factory { Effect create(Kind kind); }
    final boolean noiseSuppression, automaticGain;
    private final List<Effect> effects = new ArrayList<>();
    private boolean closed;
    AudioCaptureEffects(boolean noise, boolean gain, Factory factory) {
        noiseSuppression = noise && enable(factory, Kind.NOISE_SUPPRESSION);
        automaticGain = gain && enable(factory, Kind.AUTOMATIC_GAIN);
    }
    private boolean enable(Factory factory, Kind kind) {
        Effect effect = null;
        try {
            effect = factory.create(kind);
            if (effect != null && effect.enable()) { effects.add(effect); return true; }
        } catch (RuntimeException ignored) {}
        if (effect != null) try { effect.close(); } catch (RuntimeException ignored) {}
        return false;
    }
    @Override public void close() {
        if (closed) return;
        closed = true;
        RuntimeException failure = null;
        for (Effect effect : effects) try { effect.close(); }
        catch (RuntimeException error) { if (failure == null) failure = error; else failure.addSuppressed(error); }
        effects.clear();
        if (failure != null) throw failure;
    }
}
