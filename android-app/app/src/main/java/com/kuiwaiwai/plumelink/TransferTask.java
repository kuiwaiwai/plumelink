package com.kuiwaiwai.plumelink;

import java.io.Closeable;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.LongSupplier;

final class TransferTask {
    interface Io<T> { T run() throws Exception; }
    final String origin, owner, transport;
    final boolean upload;
    final AtomicBoolean cancelled = new AtomicBoolean();
    final AtomicReference<Closeable> active = new AtomicReference<>();
    final AtomicReference<Closeable> local = new AtomicReference<>();
    final AtomicReference<Runnable> providerCancel = new AtomicReference<>();
    final LongSupplier clock;
    volatile Thread thread;
    volatile int completed, total, count;
    volatile boolean uploadPending;
    volatile String name = "", direction = "";
    private volatile long providerStarted, cancelledAt;

    TransferTask(String origin, String owner, String transport, boolean upload) {
        this(origin, owner, transport, upload, System::nanoTime);
    }
    TransferTask(String origin, String owner, String transport, boolean upload, LongSupplier clock) {
        this.origin = origin; this.owner = owner; this.transport = transport; this.upload = upload; this.clock = clock;
    }
    void check() throws TransferFailure {
        if (cancelled.get() || Thread.currentThread().isInterrupted()) throw new TransferFailure(R.string.ui_transfer_canceled);
    }
    void uploadCompleted(String label) {
        completed++; count=completed; direction="send"; name=label; uploadPending=false;
    }
    <T> T provider(Io<T> operation) throws Exception {
        check();
        return cleanup(operation);
    }
    <T> T cleanup(Io<T> operation) throws Exception {
        providerStarted = clock.getAsLong();
        try { return operation.run(); }
        finally { providerStarted = 0; }
    }
    boolean providerTimedOut() {
        long began = providerStarted;
        return began != 0 && clock.getAsLong() - began >= TimeUnit.SECONDS.toNanos(30);
    }
    boolean cancellationExpired() {
        return cancelled.get() && clock.getAsLong() - cancelledAt >= TimeUnit.SECONDS.toNanos(6);
    }
    void providerCancellation(Runnable action) {
        providerCancel.set(action);
        if (cancelled.get()) ProviderTasks.cancel(action);
    }
    boolean cancel() {
        if (!cancelled.compareAndSet(false, true)) return false;
        cancelledAt = clock.getAsLong();
        Thread worker = thread;
        if (worker != null) worker.interrupt();
        Closeable network = active.get(), file = local.get();
        Runnable signal = providerCancel.get();
        if (network != null) ProviderTasks.cancel(() -> close(network));
        if (file != null) ProviderTasks.cancel(() -> close(file));
        if (signal != null) ProviderTasks.cancel(signal);
        return true;
    }
    private static void close(Closeable handle) {
        try { handle.close(); } catch (Exception ignored) {}
    }
}
