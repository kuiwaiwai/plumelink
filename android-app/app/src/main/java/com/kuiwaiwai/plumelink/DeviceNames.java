package com.kuiwaiwai.plumelink;

import android.content.Context;
import android.os.Build;
import android.provider.Settings;
import java.util.function.Supplier;

final class DeviceNames {
    static String read(Context context, String fallback) {
        return resolve(
            () -> Settings.Global.getString(context.getContentResolver(), Settings.Global.DEVICE_NAME),
            () -> Settings.System.getString(context.getContentResolver(), "device_name"),
            Build.MODEL, fallback);
    }

    static String resolve(Supplier<String> global, Supplier<String> system, String model, String fallback) {
        for (Supplier<String> source : java.util.Arrays.asList(global, system, () -> model)) {
            try {
                String name = clean(source.get());
                if (!name.isEmpty()) return name;
            } catch (RuntimeException ignored) {}
        }
        return fallback;
    }

    static String clean(String value) {
        if (value == null) return "";
        String name = value.trim();
        if (name.equalsIgnoreCase("null") || name.equalsIgnoreCase("unknown")) return "";
        StringBuilder text = new StringBuilder();
        name.codePoints().filter(c -> !Character.isISOControl(c)).limit(40).forEach(text::appendCodePoint);
        return text.toString().trim();
    }
}
