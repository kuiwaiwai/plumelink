package com.kuiwaiwai.plumelink;

import java.util.ArrayList;
import java.util.List;
import org.json.JSONObject;
import org.junit.Test;
import static org.junit.Assert.*;

public class BackgroundControlConnectionTest {
    private static final class Clock implements BackgroundControlConnection.Scheduler {
        long time;
        static final class Job {
            final Runnable action; final long at;
            Job(Runnable action, long at) { this.action = action; this.at = at; }
        }
        final List<Job> jobs = new ArrayList<>();
        public long now() { return time; }
        public void post(Runnable action, long delay) { jobs.add(new Job(action, time + delay)); }
        public void remove(Runnable action) { jobs.removeIf(job -> job.action == action); }
        void advance(long duration) {
            long end = time + duration;
            while (true) {
                Job next = jobs.stream().filter(job -> job.at <= end).min(java.util.Comparator.comparingLong(job -> job.at)).orElse(null);
                if (next == null) break;
                jobs.remove(next); time = next.at; next.action.run();
            }
            time = end;
        }
    }
    private static final class Socket implements BackgroundControlConnection.Connection {
        final BackgroundControlConnection.Callbacks callbacks;
        final List<String> sent = new ArrayList<>();
        boolean cancelled, pong = true;
        Socket(BackgroundControlConnection.Callbacks callbacks) { this.callbacks = callbacks; }
        public boolean send(String text) {
            if (cancelled) return false;
            sent.add(text);
            if (pong && text.equals("{\"kind\":\"ping\"}")) callbacks.message("{\"kind\":\"pong\"}");
            return true;
        }
        public void cancel() { cancelled = true; }
        void ready() {
            callbacks.opened();
            callbacks.message("{\"kind\":\"ready\"}");
            callbacks.message("{\"kind\":\"standby\"}");
        }
    }
    private static final class Fixture {
        final Clock clock = new Clock();
        final List<Socket> sockets = new ArrayList<>();
        final List<String> events = new ArrayList<>();
        final BackgroundControlConnection connection = new BackgroundControlConnection(clock, (phase, code) -> events.add(phase + ":" + code));
        void start() {
            connection.start(callbacks -> { Socket socket = new Socket(callbacks); sockets.add(socket); return socket; }, "a".repeat(64));
        }
    }

    @Test public void nativePresenceSurvivesWithoutWebViewTimersAndNeverSendsInput() throws Exception {
        Fixture f = new Fixture(); f.start(); Socket socket = f.sockets.get(0); socket.ready();
        f.clock.advance(180_000);
        assertEquals(1, f.sockets.size()); assertFalse(socket.cancelled);
        assertEquals("connected:0", f.events.get(f.events.size() - 1));
        assertEquals(182, socket.sent.size());
        for (String text : socket.sent) assertTrue(List.of("auth", "standby", "ping").contains(new JSONObject(text).getString("kind")));
        f.connection.stop(); assertTrue(f.clock.jobs.isEmpty()); assertTrue(socket.cancelled);
    }
    @Test public void foregroundReturnCancelsRetryAndIgnoresEveryLateSocketCallback() {
        Fixture f = new Fixture(); f.start(); Socket first = f.sockets.get(0); first.ready();
        first.callbacks.closed(1006); f.connection.stop();
        first.callbacks.opened(); first.callbacks.message("{\"kind\":\"ready\"}"); first.callbacks.closed(1006);
        f.clock.advance(120_000); assertEquals(1, f.sockets.size()); assertTrue(f.clock.jobs.isEmpty());
    }
    @Test public void endpointOrCredentialReplacementIgnoresOldRevocationAndOpen() {
        Fixture f = new Fixture(); f.start(); Socket first = f.sockets.get(0); f.start();
        first.ready(); first.callbacks.closed(4003);
        assertTrue(first.sent.isEmpty()); assertTrue(first.cancelled);
        Socket second = f.sockets.get(1); second.ready(); f.clock.advance(10_000);
        assertEquals(2, f.sockets.size()); assertEquals("connected:0", f.events.get(f.events.size() - 1)); f.connection.stop();
    }
    @Test public void missingPongsAndUnavailablePcUseOneBoundedBackoff() {
        Fixture f = new Fixture(); f.start(); Socket first = f.sockets.get(0); first.ready(); first.pong = false;
        f.clock.advance(4000); assertTrue(first.cancelled); assertEquals(1, f.clock.jobs.size());
        f.clock.advance(499); assertEquals(1, f.sockets.size()); f.clock.advance(1); assertEquals(2, f.sockets.size());
        f.clock.advance(60_000); assertTrue(f.sockets.size() < 12); assertEquals(1, f.clock.jobs.size());
        f.connection.stop(); assertTrue(f.clock.jobs.isEmpty());
    }
    @Test public void permissionUsbAndProtocolFailuresStopRatherThanRetryForever() {
        for (int code : new int[]{4001, 4002, 4003, 4008, 4409}) {
            Fixture f = new Fixture(); f.start(); Socket socket = f.sockets.get(0); socket.callbacks.closed(code);
            socket.callbacks.closed(1006); f.clock.advance(120_000);
            assertEquals(1, f.sockets.size()); assertTrue(f.clock.jobs.isEmpty());
            assertEquals("blocked:" + code, f.events.get(f.events.size() - 1));
        }
    }
    @Test public void standbyMustBeAcknowledgedBeforeReportingConnected() {
        Fixture f = new Fixture(); f.start(); Socket socket = f.sockets.get(0);
        socket.callbacks.opened(); socket.callbacks.message("{\"kind\":\"ready\"}");
        assertEquals("connecting:0", f.events.get(f.events.size() - 1));
        assertEquals("{\"kind\":\"standby\"}", socket.sent.get(1));
        f.clock.advance(5000); assertTrue(socket.cancelled); assertEquals("retrying:0", f.events.get(f.events.size() - 1)); f.connection.stop();
    }
    @Test public void malformedDuplicateAndUnsolicitedMessagesCannotStartInputOrRetry() {
        for (String message : List.of("[]", "not-json", "{\"kind\":\"qftOpen\"}", "{\"kind\":\"ready\"}")) {
            Fixture f = new Fixture(); f.start(); Socket socket = f.sockets.get(0); socket.ready(); socket.callbacks.message(message);
            f.clock.advance(30_000); assertEquals(1, f.sockets.size()); assertTrue(socket.cancelled);
            assertEquals("blocked:4002", f.events.get(f.events.size() - 1));
        }
    }
    @Test public void invalidCredentialsNeverLeaveAnAttemptOrTimer() {
        Fixture f = new Fixture(); f.start(); Socket socket = f.sockets.get(0);
        f.connection.start(callbacks -> { fail("invalid token must not connect"); return null; }, "invalid");
        assertTrue(socket.cancelled); assertTrue(f.clock.jobs.isEmpty());
    }
}
