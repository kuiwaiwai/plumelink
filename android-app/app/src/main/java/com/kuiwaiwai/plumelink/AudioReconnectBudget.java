package com.kuiwaiwai.plumelink;

final class AudioReconnectBudget {
    private static final int MAX_RETRIES = 2;
    private static final long STABLE_MS = 5000, MAX_LIVE_GAP_MS = 1200;
    private int retries;
    private boolean confirmed;
    private long liveSince = -1, lastLive = -1;

    void reset() { retries = 0; beginAttempt(); }

    void beginAttempt() { confirmed = false; liveSince = lastLive = -1; }

    void confirm() { confirmed = true; }

    void live(long now) {
        if (!confirmed) return;
        if (liveSince < 0 || now < lastLive || now - lastLive > MAX_LIVE_GAP_MS) liveSince = now;
        lastLive = now;
        if (now - liveSince >= STABLE_MS) retries = 0;
    }

    int retryDelayMillis() {
        beginAttempt();
        return retries < MAX_RETRIES ? ++retries * 1600 : 0;
    }
}
