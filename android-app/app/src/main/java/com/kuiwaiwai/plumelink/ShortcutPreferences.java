package com.kuiwaiwai.plumelink;

import org.json.*;

final class ShortcutPreferences {
    private ShortcutPreferences() {}
    static String normalize(String value) {
        if (value == null || value.length() > 1024) return "";
        try {
            JSONArray input = new JSONArray(value), result = new JSONArray();
            if (input.length() != 3) return "";
            for (int i = 0; i < input.length(); i++) {
                JSONObject item = input.getJSONObject(i);
                if (item.length() != 2 || !(item.opt("action") instanceof String) || !(item.opt("icon") instanceof String)) return "";
                String action = item.getString("action"), icon = item.getString("icon");
                if (!action.matches("[a-z][a-z0-9:+-]{0,95}") || !icon.matches("[a-z][a-z0-9-]{0,31}")) return "";
                result.put(new JSONObject().put("action", action).put("icon", icon));
            }
            return result.toString();
        } catch (JSONException ignored) { return ""; }
    }
}
