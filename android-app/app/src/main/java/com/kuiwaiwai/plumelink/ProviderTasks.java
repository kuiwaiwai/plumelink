package com.kuiwaiwai.plumelink;

import java.net.URI;
import java.util.Collection;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

final class ProviderTasks {
    static final ProviderTasks SHARED = new ProviderTasks(4);
    private static final ThreadPoolExecutor CANCEL = new ThreadPoolExecutor(2, 2, 30, TimeUnit.SECONDS,
        new ArrayBlockingQueue<>(8), action -> {
            Thread worker = new Thread(action, "PlumeLink-provider-cancel"); worker.setDaemon(true); return worker;
        }, new ThreadPoolExecutor.AbortPolicy());
    static { CANCEL.allowCoreThreadTimeOut(true); }
    private final int limit;
    private final Set<String> active = new HashSet<>();
    private int workers;
    ProviderTasks(int limit) {
        if (limit < 1) throw new IllegalArgumentException("Worker limit must be positive");
        this.limit = limit;
    }
    synchronized Lease tryAcquire(Collection<String> authorities) {
        Set<String> keys = new HashSet<>();
        for (String key : authorities) if (key != null && !key.isEmpty()) keys.add(key);
        if (workers >= limit || keys.stream().anyMatch(active::contains)) return null;
        workers++; active.addAll(keys);
        return new Lease(this, keys);
    }
    private synchronized void release(Set<String> keys) { active.removeAll(keys); workers--; }
    static String authority(String value) {
        try { String authority = new URI(value).getRawAuthority(); return authority == null ? "" : authority.toLowerCase(Locale.ROOT); }
        catch (Exception ignored) { return ""; }
    }
    static void cancel(Runnable action) {
        try { CANCEL.execute(() -> { try { action.run(); } catch (RuntimeException ignored) {} }); }
        catch (RuntimeException ignored) {}
    }
    static final class Lease implements AutoCloseable {
        private final ProviderTasks owner;
        private final Set<String> keys;
        private final AtomicBoolean closed = new AtomicBoolean();
        Lease(ProviderTasks owner, Set<String> keys) { this.owner = owner; this.keys = keys; }
        @Override public void close() { if (closed.compareAndSet(false, true)) owner.release(keys); }
    }
}
