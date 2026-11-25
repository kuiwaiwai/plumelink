package com.kuiwaiwai.plumelink;

import java.util.concurrent.CancellationException;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.Executor;
import java.util.concurrent.atomic.AtomicBoolean;

final class AudioCaptureResource implements AutoCloseable {
    private final Object lifecycle = new Object();
    private final AtomicBoolean closing = new AtomicBoolean();
    private final CountDownLatch closed = new CountDownLatch(1);
    private final Executor executor;
    private final Runnable release;
    private volatile RuntimeException failure;

    AudioCaptureResource(Executor executor, Runnable release) { this.executor = executor; this.release = release; }

    void start(Runnable action) {
        if (closing.get()) throw new CancellationException("麦克风启动已取消");
        synchronized (lifecycle) {
            if (closing.get()) throw new CancellationException("麦克风启动已取消");
            action.run();
        }
    }

    void closeAsync() {
        if (!closing.compareAndSet(false, true)) return;
        try { executor.execute(this::dispose); }
        catch (RuntimeException error) { failure = error; closed.countDown(); }
    }

    private void dispose() {
        try { synchronized (lifecycle) { release.run(); } }
        catch (RuntimeException error) { failure = error; }
        finally { closed.countDown(); }
    }

    @Override public void close() {
        if (closing.compareAndSet(false, true)) dispose();
        boolean interrupted = false;
        while (true) {
            try { closed.await(); break; }
            catch (InterruptedException error) { interrupted = true; }
        }
        if (interrupted) Thread.currentThread().interrupt();
        if (failure != null) throw failure;
    }
}
