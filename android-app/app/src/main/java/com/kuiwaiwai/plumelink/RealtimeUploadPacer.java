package com.kuiwaiwai.plumelink;

import java.util.concurrent.TimeUnit;
import java.util.function.BooleanSupplier;
import java.util.function.LongSupplier;

final class RealtimeUploadPacer {
    static final int QUANTUM = 16 * 1024;
    private static final long RATE = 1024 * 1024;
    private static final long MAX_WAIT = TimeUnit.MILLISECONDS.toNanos(16);
    interface Sleeper { void sleep(long nanos) throws InterruptedException; }
    private final BooleanSupplier realtime, cancelled;
    private final LongSupplier clock;
    private final Sleeper sleeper;
    private boolean scheduled;
    private long next;
    RealtimeUploadPacer(BooleanSupplier realtime, BooleanSupplier cancelled) {
        this(realtime, cancelled, System::nanoTime, TimeUnit.NANOSECONDS::sleep);
    }
    RealtimeUploadPacer(BooleanSupplier realtime, BooleanSupplier cancelled, LongSupplier clock, Sleeper sleeper) {
        this.realtime = realtime; this.cancelled = cancelled; this.clock = clock; this.sleeper = sleeper;
    }
    private void check() throws InterruptedException {
        if (cancelled.getAsBoolean() || Thread.currentThread().isInterrupted()) throw new InterruptedException("Upload cancelled");
    }
    void beforeWrite(int bytes) throws InterruptedException {
        if (bytes < 0 || bytes > QUANTUM) throw new IllegalArgumentException("Invalid upload quantum");
        check();
        if (!realtime.getAsBoolean()) { scheduled = false; return; }
        if (bytes == 0) return;
        long now = clock.getAsLong();
        next = (scheduled ? Math.max(next, now) : now) + (bytes * 1_000_000_000L + RATE - 1) / RATE;
        scheduled = true;
        while (true) {
            check();
            if (!realtime.getAsBoolean()) { scheduled = false; return; }
            long remaining = next - clock.getAsLong();
            if (remaining <= 0) return;
            try { sleeper.sleep(Math.min(remaining, MAX_WAIT)); }
            catch (InterruptedException error) { Thread.currentThread().interrupt(); throw error; }
        }
    }
}
