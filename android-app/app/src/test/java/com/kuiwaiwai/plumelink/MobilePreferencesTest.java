package com.kuiwaiwai.plumelink;

import org.json.*;
import org.junit.Test;
import static org.junit.Assert.*;

public class MobilePreferencesTest {
    static String sample() {
        return sampleV2().replace("{\"version\":2,", "{\"version\":3,\"shortcutPlans\":[],");
    }
    static String sampleV2() {
        return "{\"version\":2,\"microphoneMode\":\"voice\",\"sensitivity\":1.5,\"keepAwake\":true,\"haptics\":false,\"hardwareVolume\":false,\"longPressDrag\":true,\"scrollDirection\":\"follow\",\"responseMode\":\"balanced\",\"padOnRight\":true,\"focusMode\":false,\"controlMode\":\"dpad\",\"dialSize\":\"large\",\"shortcuts\":[{\"action\":\"copy\",\"icon\":\"heart\"},{\"action\":\"paste\",\"icon\":\"star\"},{\"action\":\"undo\",\"icon\":\"left\"}]}";
    }
    @Test public void controlsAndShortcutsRoundTripAsOneSnapshot() throws Exception {
        JSONObject restored = new JSONObject(MobilePreferences.normalize(sample()));
        assertEquals(15, restored.length());
        assertEquals(0, restored.getJSONArray("shortcutPlans").length());
        assertFalse(restored.getBoolean("haptics"));
        assertEquals("dpad", restored.getString("controlMode"));
        assertEquals("heart", restored.getJSONArray("shortcuts").getJSONObject(0).getString("icon"));
        assertEquals("voice", restored.getString("microphoneMode"));
    }
    @Test public void versionOneMigratesWithoutEnablingProcessingOrAllowingOldWritersToResetIt() throws Exception {
        JSONObject legacy = new JSONObject(sampleV2()); legacy.remove("microphoneMode"); legacy.put("version", 1);
        JSONObject migrated = new JSONObject(MobilePreferences.normalize(legacy.toString()));
        assertEquals(3, migrated.getInt("version")); assertEquals("standard", migrated.getString("microphoneMode"));
        assertEquals("dpad", migrated.getString("controlMode")); assertFalse(migrated.getBoolean("haptics"));
        assertEquals("", MobilePreferences.incoming(legacy.toString()));
        JSONObject previous = new JSONObject(MobilePreferences.normalize(sampleV2()));
        assertEquals(3, previous.getInt("version")); assertEquals("voice", previous.getString("microphoneMode"));
        assertEquals(0, previous.getJSONArray("shortcutPlans").length());
        assertEquals("", MobilePreferences.incoming(sampleV2()));
        assertEquals("", MobilePreferences.incoming(new JSONObject(sample()).put("microphoneMode", "unknown").toString()));
    }
    @Test public void malformedOutOfRangeAndUnrelatedFieldsAreRejected() throws Exception {
        for (Object sensitivity : new Object[]{.49, 3.01, "1.5", true})
            assertEquals("", MobilePreferences.normalize(new JSONObject(sample()).put("sensitivity", sensitivity).toString()));
        for (String key : new String[]{"haptics", "keepAwake", "hardwareVolume", "longPressDrag", "padOnRight", "focusMode"})
            assertEquals("", MobilePreferences.normalize(new JSONObject(sample()).put(key, "false").toString()));
        for (String key : new String[]{"controlMode", "dialSize", "responseMode", "scrollDirection"})
            assertEquals("", MobilePreferences.normalize(new JSONObject(sample()).put(key, "unknown").toString()));
        assertEquals("", MobilePreferences.normalize(new JSONObject(sample()).put("token", "secret").toString()));
        assertEquals("", MobilePreferences.normalize(new JSONObject(sample()).put("version", 4).toString()));
        JSONObject missing = new JSONObject(sample()); missing.remove("haptics");
        assertEquals("", MobilePreferences.normalize(missing.toString()));
        for (String value : new String[]{"", "[]", "{}", "x".repeat(2049)}) assertEquals("", MobilePreferences.normalize(value));
        assertEquals("", MobilePreferences.normalize(null));
    }
}
