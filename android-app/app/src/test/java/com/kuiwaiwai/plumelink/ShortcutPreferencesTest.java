package com.kuiwaiwai.plumelink;

import org.json.*;
import org.junit.Test;
import static org.junit.Assert.*;

public class ShortcutPreferencesTest {
    private String preferences() { return "[{\"action\":\"slideprevious\",\"icon\":\"heart\"},{\"action\":\"key:ctrl+shift+f5\",\"icon\":\"keyboard\"},{\"action\":\"mute\",\"icon\":\"volume\"}]"; }
    @Test public void allThreeActionsAndCustomIconsRoundTripTogether() throws Exception {
        JSONArray saved = new JSONArray(ShortcutPreferences.normalize(preferences()));
        assertEquals(3, saved.length());
        assertEquals("heart", saved.getJSONObject(0).getString("icon"));
        assertEquals("key:ctrl+shift+f5", saved.getJSONObject(1).getString("action"));
    }
    @Test public void invalidOrUnboundedPayloadsAreNotStored() {
        for (String value : new String[]{"", "{}", "[]", "null", "x".repeat(1025),
                preferences().replace("slideprevious", "../private"), preferences().replace("heart", "x".repeat(33)),
                preferences().replace("\"mute\"", "false"), preferences().replace("\"volume\"", "\"volume\",\"token\":\"secret\"")})
            assertEquals(value, "", ShortcutPreferences.normalize(value));
        assertEquals("", ShortcutPreferences.normalize(null));
    }
}
