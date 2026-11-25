package com.kuiwaiwai.plumelink;

import java.util.ArrayDeque;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.function.Consumer;

final class LanRecovery {
    record Result(boolean ready, String identity, String pin) {}
    interface Host {
        boolean current();
        void discover(Runnable update);
        Map<String,String> discovered();
        void probe(String url, Consumer<Result> result);
        void stopDiscovery();
        void cancelProbe();
        void deadline(Runnable timeout);
        void clearDeadline();
        void phase(String phase);
        void recovered(String url, String draft);
    }
    private final Host host;
    private final String identity, pin, original, draft, request;
    private final ArrayDeque<String> pending = new ArrayDeque<>();
    private final Set<String> tried = new HashSet<>();
    private boolean active, probing;
    private int probeGeneration;

    LanRecovery(Host host, String identity, String pin, String original, String draft, String request) {
        this.host = host; this.identity = identity; this.pin = pin; this.original = original;
        this.draft = draft; this.request = request;
    }
    String request() { return request; }
    void start() {
        if (active) return;
        active = true;
        try {
            host.phase("searching");
            host.deadline(() -> { if (active) { stop(); host.phase("error"); } });
            enqueue(original);
            host.discover(this::changed);
            advance();
        } catch (RuntimeException error) { stop(); host.phase("error"); }
    }
    private void enqueue(String url) {
        if (url != null && !url.isEmpty() && tried.size() < 8 && tried.add(url)) pending.add(url);
    }
    private void changed() {
        if (!active) return;
        if (!host.current()) { cancel(); return; }
        for (Map.Entry<String,String> candidate : host.discovered().entrySet()) {
            if (identity.equals(candidate.getValue())) enqueue(candidate.getKey());
        }
        advance();
    }
    private void advance() {
        if (!active || probing) return;
        if (!host.current()) { cancel(); return; }
        String url = pending.poll();
        if (url == null) return;
        probing = true;
        int generation = ++probeGeneration;
        host.probe(url, result -> {
            if (!active || generation != probeGeneration) return;
            probing = false;
            if (!host.current()) { cancel(); return; }
            if (result.ready() && identity.equals(result.identity()) && !pin.isEmpty() && pin.equals(result.pin())) {
                stop(); host.recovered(url, draft);
            } else advance();
        });
    }
    void cancel() { if (active) { stop(); host.phase("cancelled"); } }
    private void stop() {
        active = false; probing = false; probeGeneration++;
        pending.clear(); host.clearDeadline(); host.stopDiscovery(); host.cancelProbe();
    }
    static boolean validDraft(String value) {
        return value != null && value.length() <= 1000 && value.chars().noneMatch(ch ->
            ch == 0x7f || ch < 0x20 && ch != '\n' && ch != '\r' && ch != '\t');
    }
    static final class Draft<S> {
        private S source;
        private String identity = "", owner = "", value = "";
        synchronized void set(S target, String identity, String owner, String value) {
            this.source = target; this.identity = identity; this.owner = owner; this.value = value;
        }
        synchronized String take(S current, String identity, String owner) {
            if (source == null || source != current || !this.identity.equals(identity) || !this.owner.equals(owner)) return "";
            String result = value; clear(); return result;
        }
        synchronized void clear() { source = null; identity = ""; owner = ""; value = ""; }
    }
}
