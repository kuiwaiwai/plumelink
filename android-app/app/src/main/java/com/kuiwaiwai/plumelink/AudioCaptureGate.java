package com.kuiwaiwai.plumelink;

import java.util.concurrent.CancellationException;
import java.util.concurrent.Semaphore;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.BooleanSupplier;

final class AudioCaptureGate {
    private final Semaphore available = new Semaphore(1);
    private final long waitMillis;
    private volatile boolean failed;

    AudioCaptureGate() { this(900); }
    AudioCaptureGate(long waitMillis) { this.waitMillis = Math.max(0, waitMillis); }

    Lease acquire(BooleanSupplier cancelled) throws InterruptedException {
        long deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(waitMillis);
        do {
            if (cancelled.getAsBoolean()) throw new CancellationException("麦克风启动已取消");
            if (failed) throw new IllegalStateException("手机录音设备释放失败，请重新打开应用");
            long remaining = Math.max(0, deadline - System.nanoTime());
            if (available.tryAcquire(Math.min(remaining, TimeUnit.MILLISECONDS.toNanos(25)), TimeUnit.NANOSECONDS)) {
                if (cancelled.getAsBoolean()) { available.release(); throw new CancellationException("麦克风启动已取消"); }
                return new Lease(this);
            }
        } while (System.nanoTime() < deadline);
        throw new IllegalStateException("麦克风正在停止，请稍后重试");
    }

    static final class Lease implements AutoCloseable {
        private final AudioCaptureGate owner;
        private final AtomicBoolean finished = new AtomicBoolean();
        Lease(AudioCaptureGate owner) { this.owner = owner; }
        @Override public void close() { if (finished.compareAndSet(false, true)) owner.available.release(); }
        void fail() { if (finished.compareAndSet(false, true)) owner.failed = true; }
    }
}
