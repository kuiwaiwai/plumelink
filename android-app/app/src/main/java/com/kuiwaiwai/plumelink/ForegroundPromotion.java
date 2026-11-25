package com.kuiwaiwai.plumelink;

import java.util.function.IntConsumer;

final class ForegroundPromotion {
    private ForegroundPromotion() {}
    static boolean covers(int active, int requested) { return active != 0 && (active & requested) == requested; }
    static boolean shouldRetireIdle(boolean hasTasks, int active, int requested) {
        return !hasTasks && !covers(active, requested);
    }
    static int apply(int active, int requested, IntConsumer promote, Runnable notify) {
        if (active != requested) {
            try { promote.accept(requested); return requested; }
            catch (RuntimeException ignored) { /* Keep only types already accepted by Android. */ }
        }
        try { notify.run(); } catch (RuntimeException ignored) {}
        return active;
    }
}
