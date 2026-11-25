package com.kuiwaiwai.plumelink;

import org.json.JSONObject;

/** Maintains presence without granting a background input lease. */
final class BackgroundControlConnection {
    interface Scheduler {
        long now();
        void post(Runnable action, long delay);
        void remove(Runnable action);
    }
    interface Connection {
        boolean send(String text);
        void cancel();
    }
    interface Callbacks {
        void opened();
        void message(String text);
        void closed(int code);
    }
    interface Dialer { Connection connect(Callbacks callbacks) throws Exception; }
    interface Listener { void changed(String phase, int terminalCode); }

    private final Scheduler scheduler;
    private final Listener listener;
    private Dialer dialer;
    private String auth = "";
    private boolean active;
    private int failures;
    private Attempt current;
    private final Runnable retry = this::connect;
    private final Runnable heartbeat = this::tick;

    BackgroundControlConnection(Scheduler scheduler, Listener listener) {
        this.scheduler = scheduler;
        this.listener = listener;
    }

    void start(Dialer next, String token) {
        stop();
        if (next == null || token == null || !token.matches("[a-f0-9]{64}")) return;
        dialer = next;
        auth = "{\"kind\":\"auth\",\"token\":\"" + token + "\"}";
        active = true;
        failures = 0;
        connect();
    }

    void stop() {
        active = false;
        scheduler.remove(retry);
        scheduler.remove(heartbeat);
        discard();
        dialer = null;
        auth = "";
    }

    private void discard() {
        Attempt previous = current;
        current = null;
        if (previous != null && previous.connection != null) previous.connection.cancel();
    }

    private void connect() {
        if (!active || current != null) return;
        Attempt attempt = new Attempt();
        current = attempt;
        listener.changed(failures == 0 ? "connecting" : "retrying", 0);
        try {
            attempt.connection = dialer.connect(attempt);
            if (attempt.connection == null) throw new IllegalStateException("Connection unavailable");
            scheduler.post(heartbeat, 1000);
        } catch (Exception error) {
            attempt.closed(1006);
        }
    }

    private void tick() {
        Attempt attempt = current;
        if (!active || attempt == null) return;
        long now = scheduler.now();
        if ((!attempt.ready && now - attempt.began >= 5000)
                || (attempt.ready && now - attempt.lastPong >= 4000)) {
            attempt.closed(1006);
            return;
        }
        if (attempt.ready && !attempt.connection.send("{\"kind\":\"ping\"}")) {
            attempt.closed(1006);
            return;
        }
        scheduler.post(heartbeat, 1000);
    }

    private final class Attempt implements Callbacks {
        final long began = scheduler.now();
        long lastPong;
        boolean ready, opened, authenticated;
        Connection connection;
        boolean current() { return active && current == this; }

        @Override public void opened() {
            if (!current()) return;
            if (opened || connection == null || !connection.send(auth)) { closed(1006); return; }
            opened = true;
        }

        @Override public void message(String text) {
            if (!current()) return;
            try {
                if (!opened || text == null || text.length() > 65_536) throw new IllegalArgumentException();
                String kind = new JSONObject(text).getString("kind");
                if ("ready".equals(kind) && !authenticated) {
                    authenticated = true;
                    if (!connection.send("{\"kind\":\"standby\"}")) closed(1006);
                } else if ("standby".equals(kind) && authenticated && !ready) {
                    ready = true;
                    failures = 0;
                    lastPong = scheduler.now();
                    listener.changed("connected", 0);
                } else if ("pong".equals(kind) && ready) {
                    lastPong = scheduler.now();
                } else {
                    closed(4002);
                }
            } catch (Exception error) {
                closed(4002);
            }
        }

        @Override public void closed(int code) {
            if (!current()) return;
            scheduler.remove(heartbeat);
            discard();
            if (code == 4001 || code == 4002 || code == 4003 || code == 4008 || code == 4409) {
                active = false;
                dialer = null;
                auth = "";
                listener.changed("blocked", code);
                return;
            }
            failures = Math.min(failures + 1, 5);
            listener.changed("retrying", 0);
            scheduler.post(retry, Math.min(8000L, 500L << (failures - 1)));
        }
    }
}
