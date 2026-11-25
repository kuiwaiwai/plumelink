package com.kuiwaiwai.plumelink;

import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicBoolean;
import org.junit.Test;
import static org.junit.Assert.*;

public class AudioCaptureResourceTest {
    private static ExecutorService worker() {
        return Executors.newCachedThreadPool(task -> { Thread thread = new Thread(task); thread.setDaemon(true); return thread; });
    }
    private static void await(CountDownLatch latch) {
        try { assertTrue(latch.await(3, TimeUnit.SECONDS)); }
        catch (InterruptedException error) { Thread.currentThread().interrupt(); throw new AssertionError(error); }
    }

    @Test public void delayedReleaseDoesNotBlockStopAndPreventsOverlappingCapture() throws Exception {
        ExecutorService worker = worker();
        CountDownLatch releasing = new CountDownLatch(1), finishRelease = new CountDownLatch(1), waiting = new CountDownLatch(1);
        AtomicInteger releases = new AtomicInteger();
        AudioCaptureGate gate = new AudioCaptureGate(30);
        AudioCaptureGate.Lease lease = gate.acquire(() -> false);
        AudioCaptureResource resource = new AudioCaptureResource(worker, () -> {
            releases.incrementAndGet(); releasing.countDown(); await(finishRelease);
        });
        try {
            worker.submit(resource::closeAsync).get(1, TimeUnit.SECONDS);
            await(releasing);
            resource.closeAsync();
            Future<?> cleanup = worker.submit(() -> { waiting.countDown(); resource.close(); lease.close(); });
            await(waiting);
            assertThrows(TimeoutException.class, () -> cleanup.get(50, TimeUnit.MILLISECONDS));
            assertThrows(IllegalStateException.class, () -> gate.acquire(() -> false));
            finishRelease.countDown(); cleanup.get(2, TimeUnit.SECONDS);
            try (AudioCaptureGate.Lease next = gate.acquire(() -> false)) { assertNotNull(next); }
            assertEquals(1, releases.get());
        } finally { finishRelease.countDown(); worker.shutdownNow(); }
    }

    @Test public void stopDuringNativeStartDoesNotWaitForStartLock() throws Exception {
        ExecutorService worker = worker();
        CountDownLatch starting = new CountDownLatch(1), finishStart = new CountDownLatch(1);
        AtomicInteger releases = new AtomicInteger();
        AudioCaptureResource resource = new AudioCaptureResource(worker, releases::incrementAndGet);
        try {
            Future<?> start = worker.submit(() -> resource.start(() -> { starting.countDown(); await(finishStart); }));
            await(starting);
            worker.submit(resource::closeAsync).get(1, TimeUnit.SECONDS);
            assertEquals(0, releases.get());
            assertThrows(CancellationException.class, () -> resource.start(() -> fail("closed resource restarted")));
            finishStart.countDown(); start.get(2, TimeUnit.SECONDS);
            resource.close();
            assertEquals(1, releases.get());
        } finally { finishStart.countDown(); worker.shutdownNow(); }
    }

    @Test public void releaseFailureIsRetainedAndBlocksNewHardwareAllocation() throws Exception {
        AudioCaptureGate gate = new AudioCaptureGate(0);
        AudioCaptureGate.Lease lease = gate.acquire(() -> false);
        IllegalStateException failure = new IllegalStateException("native release failed");
        AtomicInteger releases = new AtomicInteger();
        AudioCaptureResource resource = new AudioCaptureResource(Runnable::run, () -> { releases.incrementAndGet(); throw failure; });
        resource.closeAsync();
        assertSame(failure, assertThrows(IllegalStateException.class, resource::close));
        lease.fail(); lease.close();
        assertThrows(IllegalStateException.class, () -> gate.acquire(() -> false));
        resource.closeAsync();
        assertSame(failure, assertThrows(IllegalStateException.class, resource::close));
        assertEquals(1, releases.get());
    }

    @Test public void rejectedStopTaskDoesNotRunNativeReleaseOnCaller() {
        AtomicInteger releases = new AtomicInteger();
        AudioCaptureResource resource = new AudioCaptureResource(task -> { throw new RejectedExecutionException(); }, releases::incrementAndGet);
        resource.closeAsync();
        assertThrows(RejectedExecutionException.class, resource::close);
        assertEquals(0, releases.get());
    }

    @Test public void cancelledWaitReleasesNoOtherSessionsLease() throws Exception {
        AudioCaptureGate gate = new AudioCaptureGate(0);
        AudioCaptureGate.Lease first = gate.acquire(() -> false);
        assertThrows(CancellationException.class, () -> gate.acquire(() -> true));
        assertThrows(IllegalStateException.class, () -> gate.acquire(() -> false));
        first.close(); first.close();
        try (AudioCaptureGate.Lease second = gate.acquire(() -> false)) {
            assertThrows(IllegalStateException.class, () -> gate.acquire(() -> false));
        }
        AtomicInteger checks = new AtomicInteger();
        assertThrows(CancellationException.class, () -> gate.acquire(() -> checks.incrementAndGet() == 2));
        try (AudioCaptureGate.Lease next = gate.acquire(() -> false)) { assertNotNull(next); }
    }

    @Test public void stoppedQueuedStartCancelsWithoutWaitingForHardwareTimeout() throws Exception {
        ExecutorService worker = worker();
        AudioCaptureGate gate = new AudioCaptureGate(2000);
        AudioCaptureGate.Lease owner = gate.acquire(() -> false);
        AtomicBoolean cancelled = new AtomicBoolean();
        CountDownLatch waiting = new CountDownLatch(1);
        try {
            Future<?> next = worker.submit(() -> {
                waiting.countDown();
                assertThrows(CancellationException.class, () -> gate.acquire(cancelled::get));
            });
            await(waiting);
            assertThrows(TimeoutException.class, () -> next.get(30, TimeUnit.MILLISECONDS));
            cancelled.set(true); next.get(1, TimeUnit.SECONDS);
        } finally { cancelled.set(true); owner.close(); worker.shutdownNow(); }
    }
}
