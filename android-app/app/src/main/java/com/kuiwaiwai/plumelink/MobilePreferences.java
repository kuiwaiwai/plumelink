package com.kuiwaiwai.plumelink;

import org.json.*;

final class MobilePreferences {
    private MobilePreferences() {}
    private static final String[] FLAGS = {"keepAwake", "haptics", "hardwareVolume", "longPressDrag", "padOnRight", "focusMode"};
    static String normalize(String value) {
        return normalize(value, true);
    }
    static String incoming(String value) {
        return normalize(value, false);
    }
    private static String normalize(String value, boolean migrate) {
        if (value == null || value.length() > 16384) return "";
        try {
            JSONObject input = new JSONObject(value), result = new JSONObject();
            if (!(input.opt("version") instanceof Number) || !(input.opt("sensitivity") instanceof Number)) return "";
            double version = input.getDouble("version");
            boolean legacy = migrate && version == 1 && input.length() == 13;
            boolean previous = migrate && version == 2 && input.length() == 14;
            if (!legacy && !previous && (version != 3 || input.length() != 15)) return "";
            result.put("version", 3);
            String plans = legacy || previous ? "[]" : ShortcutPlans.normalize(input.opt("shortcutPlans"));
            if (plans.isEmpty()) return "";
            result.put("shortcutPlans", new JSONArray(plans));
            if (legacy) result.put("microphoneMode", "standard");
            else if (!choice(input, result, "microphoneMode", "standard", "voice")) return "";
            double sensitivity = input.getDouble("sensitivity");
            if (!Double.isFinite(sensitivity) || sensitivity < .5 || sensitivity > 3) return "";
            result.put("sensitivity", sensitivity);
            for (String flag : FLAGS) {
                if (!(input.opt(flag) instanceof Boolean)) return "";
                result.put(flag, input.getBoolean(flag));
            }
            if (!choice(input, result, "scrollDirection", "follow", "opposite")
                || !choice(input, result, "responseMode", "balanced", "responsive")
                || !choice(input, result, "controlMode", "trackpad", "dpad")
                || !choice(input, result, "dialSize", "small", "standard", "large")) return "";
            String shortcuts = ShortcutPreferences.normalize(String.valueOf(input.opt("shortcuts")));
            if (shortcuts.isEmpty()) return "";
            return result.put("shortcuts", new JSONArray(shortcuts)).toString();
        } catch (JSONException ignored) { return ""; }
    }
    private static boolean choice(JSONObject input, JSONObject result, String key, String... allowed) throws JSONException {
        Object value = input.opt(key);
        for (String option : allowed) if (option.equals(value)) { result.put(key, value); return true; }
        return false;
    }
}
