package com.kuiwaiwai.plumelink;

import org.json.*;
import java.text.Normalizer;
import java.util.HashSet;
import java.util.Locale;

final class ShortcutPlans {
    private ShortcutPlans() {}
    static String normalize(Object value) {
        if (!(value instanceof JSONArray input) || input.length() > 8) return "";
        JSONArray result = new JSONArray();
        HashSet<String> ids = new HashSet<>(), names = new HashSet<>();
        try {
            for (int i = 0; i < input.length(); i++) {
                JSONObject item = input.getJSONObject(i);
                if (item.length() != 3 || !(item.opt("id") instanceof String) || !(item.opt("name") instanceof String)) return "";
                String id = item.getString("id"), name = item.getString("name");
                if (!id.matches("p[a-f0-9]{32}") || !ids.add(id) || name.isEmpty() || name.codePointCount(0, name.length()) > 32
                    || !name.equals(name.trim()) || !name.equals(Normalizer.normalize(name, Normalizer.Form.NFC))
                    || name.matches("(?s).*[\\x00-\\x1f\\x7f-\\x9f\\u202a-\\u202e\\u2066-\\u2069].*")
                    || !names.add(name.toLowerCase(Locale.ROOT))) return "";
                String shortcuts = ShortcutPreferences.normalize(String.valueOf(item.opt("shortcuts")));
                if (shortcuts.isEmpty()) return "";
                JSONArray slots = new JSONArray(shortcuts);
                HashSet<String> actions = new HashSet<>();
                for (int j = 0; j < slots.length(); j++) if (!actions.add(slots.getJSONObject(j).getString("action"))) return "";
                result.put(new JSONObject().put("id", id).put("name", name).put("shortcuts", slots));
            }
            return result.toString();
        } catch (JSONException ignored) { return ""; }
    }
}
