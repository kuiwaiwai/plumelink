package com.kuiwaiwai.plumelink;

import java.io.ByteArrayOutputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import org.junit.Test;
import static org.junit.Assert.*;

public class RealtimeUploadPacerTest {
    private static final long QUANTUM_NANOS = 15_625_000;
    @Test public void realtimeWritesPayForEachQuantumBeforeItIsSent() throws Exception {
        AtomicLong now = new AtomicLong(); List<Long> waits = new ArrayList<>();
        RealtimeUploadPacer pacer = new RealtimeUploadPacer(() -> true, () -> false, now::get,
            nanos -> { waits.add(nanos); now.addAndGet(nanos); });
        pacer.beforeWrite(RealtimeUploadPacer.QUANTUM); assertEquals(QUANTUM_NANOS, now.get());
        pacer.beforeWrite(RealtimeUploadPacer.QUANTUM); assertEquals(2 * QUANTUM_NANOS, now.get());
        assertTrue(waits.stream().allMatch(nanos -> nanos > 0 && nanos <= TimeUnit.MILLISECONDS.toNanos(16)));
    }
    @Test public void microphoneChangesInsideALargeProtocolBlockApplyToTheNextActualWrite() throws Exception {
        byte[] source = new byte[4 * 1024 * 1024]; new java.util.Random(61).nextBytes(source);
        AtomicBoolean realtime = new AtomicBoolean(); AtomicLong now = new AtomicLong(); AtomicInteger writes = new AtomicInteger();
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        RealtimeUploadPacer pacer = new RealtimeUploadPacer(realtime::get, () -> false, now::get, now::addAndGet);
        ResumableUpload.writePaced(source, source.length, pacer, (bytes, offset, count) -> {
            int write = writes.incrementAndGet(); assertTrue(count <= RealtimeUploadPacer.QUANTUM);
            if (write == 1) { assertEquals(0, now.get()); realtime.set(true); }
            if (write == 2) assertEquals(QUANTUM_NANOS, now.get());
            if (write == 128) realtime.set(false);
            output.write(bytes, offset, count);
        });
        assertArrayEquals(source, output.toByteArray()); assertEquals(256, writes.get());
        assertEquals(127 * QUANTUM_NANOS, now.get());
    }
    @Test public void stoppingMicrophoneReleasesTheWaitAndDoesNotCarryDebtIntoTheNextSession() throws Exception {
        AtomicLong now = new AtomicLong(); AtomicBoolean realtime = new AtomicBoolean(true), stopDuringSleep = new AtomicBoolean(true);
        RealtimeUploadPacer pacer = new RealtimeUploadPacer(realtime::get, () -> false, now::get, nanos -> {
            if (stopDuringSleep.get()) { now.addAndGet(TimeUnit.MILLISECONDS.toNanos(4)); realtime.set(false); }
            else now.addAndGet(nanos);
        });
        pacer.beforeWrite(RealtimeUploadPacer.QUANTUM); assertEquals(TimeUnit.MILLISECONDS.toNanos(4), now.get());
        stopDuringSleep.set(false); realtime.set(true); long before = now.get();
        pacer.beforeWrite(RealtimeUploadPacer.QUANTUM); assertEquals(QUANTUM_NANOS, now.get() - before);
    }
    @Test public void cancellationDuringPacingPreventsThePendingWrite() throws Exception {
        AtomicLong now = new AtomicLong(); AtomicBoolean cancelled = new AtomicBoolean(); AtomicInteger writes = new AtomicInteger();
        RealtimeUploadPacer pacer = new RealtimeUploadPacer(() -> true, cancelled::get, now::get,
            nanos -> { now.addAndGet(nanos); cancelled.set(true); });
        try {
            TransferFailure failure = assertThrows(TransferFailure.class,
                () -> ResumableUpload.writePaced(new byte[32768], 32768, pacer, (bytes, offset, count) -> writes.incrementAndGet()));
            assertEquals(R.string.ui_transfer_canceled, failure.reason()); assertEquals(0, writes.get());
            assertTrue(Thread.currentThread().isInterrupted());
        } finally { Thread.interrupted(); }
    }
    @Test public void slowNetworkDoesNotCauseCatchupBurstsOrGrowingPacingDebt() throws Exception {
        AtomicLong now = new AtomicLong();
        RealtimeUploadPacer pacer = new RealtimeUploadPacer(() -> true, () -> false, now::get, now::addAndGet);
        pacer.beforeWrite(RealtimeUploadPacer.QUANTUM); now.addAndGet(TimeUnit.SECONDS.toNanos(1));
        long before = now.get(); pacer.beforeWrite(RealtimeUploadPacer.QUANTUM);
        assertEquals(QUANTUM_NANOS, now.get() - before);
    }
    @Test public void zeroBytesDoNotWaitAndExistingInterruptionIsNotConsumed() throws Exception {
        AtomicInteger sleeps = new AtomicInteger();
        RealtimeUploadPacer pacer = new RealtimeUploadPacer(() -> true, () -> false, () -> 0, nanos -> sleeps.incrementAndGet());
        pacer.beforeWrite(0); assertEquals(0, sleeps.get());
        Thread.currentThread().interrupt();
        try { assertThrows(InterruptedException.class, () -> pacer.beforeWrite(0)); assertTrue(Thread.currentThread().isInterrupted()); }
        finally { Thread.interrupted(); }
        assertThrows(IllegalArgumentException.class, () -> pacer.beforeWrite(RealtimeUploadPacer.QUANTUM + 1));
    }
}
