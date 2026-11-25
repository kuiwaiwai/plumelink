package com.kuiwaiwai.plumelink;

import java.util.function.Consumer;
import java.util.function.Supplier;

final class BridgeAccess<S> {
    private final Supplier<S> current;
    private final Consumer<Runnable> dispatch;

    BridgeAccess(Supplier<S> current, Consumer<Runnable> dispatch) {
        this.current = current;
        this.dispatch = dispatch;
    }

    private boolean accepts(S source) { return source != null && source == current.get(); }

    void run(S source, Runnable action) {
        dispatch.accept(() -> { if (accepts(source)) action.run(); });
    }

    <T> T read(S source, Supplier<T> value, T fallback) {
        if (!accepts(source)) return fallback;
        T result = value.get();
        return accepts(source) ? result : fallback;
    }
}
