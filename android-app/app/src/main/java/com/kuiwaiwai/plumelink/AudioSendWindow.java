package com.kuiwaiwai.plumelink;

/** Tracks only outstanding audio; an ACK cannot renew the deadline of another frame. */
final class AudioSendWindow {
    static final int CAPACITY = 4;
    static final int FRAME_BYTES = 972;
    static final long TIMEOUT_MS = 1200;
    private final long[] sentAt = new long[CAPACITY];
    private int sequence, acknowledged;

    synchronized int reserve(long now, long queuedBytes) {
        if (sequence - acknowledged >= CAPACITY || queuedBytes >= (long) FRAME_BYTES * CAPACITY) return 0;
        if (sequence == Integer.MAX_VALUE) throw new IllegalStateException("音频会话已到期，请重新开启");
        sentAt[sequence % CAPACITY] = now;
        return ++sequence;
    }

    synchronized boolean acknowledge(int ack) {
        if (ack <= acknowledged || ack > sequence) return false;
        acknowledged = ack;
        return true;
    }

    synchronized boolean expired(long now) {
        return sequence > acknowledged && now - sentAt[acknowledged % CAPACITY] >= TIMEOUT_MS;
    }
}
