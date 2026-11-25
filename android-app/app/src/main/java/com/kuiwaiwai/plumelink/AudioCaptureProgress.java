package com.kuiwaiwai.plumelink;

final class AudioCaptureProgress {
    static final long PREPARATION_TIMEOUT_MS = 3000;
    static final long DATA_TIMEOUT_MS = 1200;
    enum Timeout { NONE, PREPARATION, DATA }
    private long preparingAt = -1, readingAt = -1;
    private boolean finished;

    synchronized void prepare(long now) {
        if (!finished && preparingAt < 0) preparingAt = now;
    }

    synchronized boolean beginReading(long now) {
        if (finished || preparingAt < 0 || readingAt >= 0 || now - preparingAt >= PREPARATION_TIMEOUT_MS) return false;
        readingAt = now;
        return true;
    }

    synchronized boolean progress(long now) {
        if (finished || readingAt < 0 || now < readingAt || now - readingAt >= DATA_TIMEOUT_MS) return false;
        readingAt = now;
        return true;
    }

    synchronized Timeout timeout(long now) {
        if (finished || preparingAt < 0) return Timeout.NONE;
        if (readingAt < 0) return now - preparingAt >= PREPARATION_TIMEOUT_MS ? Timeout.PREPARATION : Timeout.NONE;
        return now - readingAt >= DATA_TIMEOUT_MS ? Timeout.DATA : Timeout.NONE;
    }

    synchronized void finish() { finished = true; preparingAt = readingAt = -1; }
}
