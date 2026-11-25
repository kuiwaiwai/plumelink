package com.kuiwaiwai.plumelink;

import java.util.HashMap;
import java.util.Map;
import java.util.function.BiConsumer;

final class HardwareVolumeKeys {
    private final Map<Integer, String> captured = new HashMap<>();
    private final BiConsumer<String, Boolean> send;

    HardwareVolumeKeys(BiConsumer<String, Boolean> send) { this.send = send; }

    boolean down(int key, boolean repeated, String action) {
        if (captured.containsKey(key)) {
            if (!repeated && captured.get(key) == null) captured.remove(key);
            else return true;
        }
        if (repeated || action == null || action.isEmpty()) return false;
        captured.put(key, action);
        send.accept(action, true);
        return true;
    }

    boolean up(int key) {
        if (!captured.containsKey(key)) return false;
        String action = captured.remove(key);
        if (action != null) send.accept(action, false);
        return true;
    }

    void cancel() {
        for (var entry : captured.entrySet()) {
            String action = entry.getValue();
            entry.setValue(null);
            if (action != null) send.accept(action, false);
        }
    }

    void reset() { cancel(); captured.clear(); }
}
