package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;
import java.util.List;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;

public class TransferTaskTest {
    private static void uninterruptible(CountDownLatch gate) {
        boolean interrupted=false;
        while (true) try { gate.await(); break; } catch (InterruptedException ignored) { interrupted=true; }
        if (interrupted) Thread.currentThread().interrupt();
    }
    @Test public void cancellationRetainsBusyProviderLeaseUntilActualReturnAndStopsLateEffects() throws Exception {
        ProviderTasks pool=new ProviderTasks(2);
        ProviderTasks.Lease held=pool.tryAcquire(List.of("bad")); assertNotNull(held);
        TransferTask old=new TransferTask("pc","owner","usb",true);
        CountDownLatch entered=new CountDownLatch(1), resume=new CountDownLatch(1);
        AtomicInteger effects=new AtomicInteger(); AtomicReference<Throwable> failure=new AtomicReference<>();
        old.thread=new Thread(() -> {
            try (held) { old.provider(() -> {entered.countDown();uninterruptible(resume);return null;}); old.check();effects.incrementAndGet(); }
            catch(Throwable error) {failure.set(error);}
        });
        old.thread.start();
        try {
            assertTrue(entered.await(2,TimeUnit.SECONDS)); assertTrue(old.cancel());
            assertNull(pool.tryAcquire(List.of("bad")));
            try(ProviderTasks.Lease healthy=pool.tryAcquire(List.of("good"))) {assertNotNull(healthy);}
            TransferTask fresh=new TransferTask("pc","owner","usb",true);fresh.check();fresh.completed=2;
            assertFalse(fresh.cancelled.get());assertEquals(0,old.completed);
        } finally {resume.countDown();old.thread.join(2000);}
        assertFalse(old.thread.isAlive());assertTrue(failure.get() instanceof TransferFailure);assertEquals(0,effects.get());
        try(ProviderTasks.Lease available=pool.tryAcquire(List.of("bad"))) {assertNotNull(available);}
    }
    @Test public void providerAndCancelDeadlinesDoNotTreatInterruptAsCompletion() throws Exception {
        AtomicLong clock=new AtomicLong(100);
        TransferTask task=new TransferTask("pc","owner","usb",false,clock::get);
        CountDownLatch entered=new CountDownLatch(1),resume=new CountDownLatch(1);
        task.thread=new Thread(() -> {try{task.provider(() -> {entered.countDown();uninterruptible(resume);return null;});}catch(Exception ignored){}});
        task.thread.start();
        try {
            assertTrue(entered.await(2,TimeUnit.SECONDS)); clock.addAndGet(TimeUnit.SECONDS.toNanos(29));assertFalse(task.providerTimedOut());
            clock.addAndGet(TimeUnit.SECONDS.toNanos(1));assertTrue(task.providerTimedOut());assertTrue(task.cancel());
            clock.addAndGet(TimeUnit.SECONDS.toNanos(5));assertFalse(task.cancellationExpired());
            clock.addAndGet(TimeUnit.SECONDS.toNanos(1));assertTrue(task.cancellationExpired());assertTrue(task.thread.isAlive());
        } finally {resume.countDown();task.thread.join(2000);}
        assertFalse(task.providerTimedOut());
    }
    @Test public void cancellationDoesNotWaitForBlockingCloseAndIsIdempotent() throws Exception {
        TransferTask task=new TransferTask("pc","owner","usb",true);
        CountDownLatch closing=new CountDownLatch(1),resume=new CountDownLatch(1),done=new CountDownLatch(1);
        AtomicInteger calls=new AtomicInteger();
        task.local.set(() -> {calls.incrementAndGet();closing.countDown();uninterruptible(resume);done.countDown();});
        try {
            assertTrue(task.cancel());assertFalse(task.cancel());
            assertTrue(closing.await(2,TimeUnit.SECONDS));assertEquals(1,calls.get());assertEquals(1,done.getCount());
        } finally {resume.countDown();}
        assertTrue(done.await(2,TimeUnit.SECONDS));
    }
}
