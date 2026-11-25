package com.kuiwaiwai.plumelink;

import java.util.Arrays;
import java.util.Collections;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.Test;
import static org.junit.Assert.*;

public class ProviderTasksTest {
    @Test public void authoritiesAreExclusiveButIndependentProvidersCanProceed() {
        ProviderTasks tasks = new ProviderTasks(2);
        ProviderTasks.Lease first = tasks.tryAcquire(Arrays.asList("one", "one", ""));
        assertNotNull(first); assertNull(tasks.tryAcquire(Collections.singleton("one")));
        ProviderTasks.Lease second = tasks.tryAcquire(Collections.singleton("two"));
        assertNotNull(second); assertNull(tasks.tryAcquire(Collections.singleton("three")));
        first.close(); first.close();
        ProviderTasks.Lease third = tasks.tryAcquire(Collections.singleton("one"));
        assertNotNull(third); assertNull(tasks.tryAcquire(Collections.singleton("four")));
        second.close(); third.close();
    }
    @Test public void cancellingAStuckWorkerDoesNotReleaseItsAuthorityEarly() throws Exception {
        ProviderTasks tasks = new ProviderTasks(2); CountDownLatch entered = new CountDownLatch(1), release = new CountDownLatch(1);
        ProviderTasks.Lease lease = tasks.tryAcquire(Collections.singleton("slow"));
        Thread worker = new Thread(() -> { try (lease) { entered.countDown(); try { release.await(); } catch (InterruptedException ignored) {} } });
        worker.start();
        try {
            assertTrue(entered.await(2, TimeUnit.SECONDS));
            assertNull(tasks.tryAcquire(Collections.singleton("slow")));
            try (ProviderTasks.Lease healthy = tasks.tryAcquire(Collections.singleton("healthy"))) { assertNotNull(healthy); }
        } finally { release.countDown(); worker.join(2000); }
        try (ProviderTasks.Lease available = tasks.tryAcquire(Collections.singleton("slow"))) { assertNotNull(available); }
    }
    @Test public void malformedAndFileUrisHaveNoProviderKey() {
        assertEquals("example.documents", ProviderTasks.authority("content://EXAMPLE.documents/tree/path"));
        assertEquals("10@example.documents", ProviderTasks.authority("content://10@example.documents/tree/path"));
        assertEquals("", ProviderTasks.authority("file:///data/file"));
        assertEquals("", ProviderTasks.authority("not a valid uri"));
        ProviderTasks tasks = new ProviderTasks(1);
        try (ProviderTasks.Lease lease = tasks.tryAcquire(Arrays.asList("", null))) {
            assertNotNull(lease); assertNull(tasks.tryAcquire(Collections.emptyList()));
        }
    }
    @Test public void cancellationLaneIsBoundedAndNeverRunsRejectedWorkOnTheCaller() throws Exception {
        CountDownLatch entered = new CountDownLatch(2), release = new CountDownLatch(1), drained = new CountDownLatch(8);
        AtomicBoolean rejected = new AtomicBoolean(); AtomicInteger queued = new AtomicInteger();
        Runnable blocked = () -> { entered.countDown(); try { release.await(); } catch (InterruptedException error) { Thread.currentThread().interrupt(); } };
        ProviderTasks.cancel(blocked); ProviderTasks.cancel(blocked);
        try {
            assertTrue(entered.await(2, TimeUnit.SECONDS));
            for (int i = 0; i < 8; i++) ProviderTasks.cancel(() -> { queued.incrementAndGet(); drained.countDown(); });
            ProviderTasks.cancel(() -> rejected.set(true));
            assertFalse(rejected.get());
        } finally { release.countDown(); }
        assertTrue(drained.await(2, TimeUnit.SECONDS)); assertEquals(8, queued.get()); assertFalse(rejected.get());
    }
}
