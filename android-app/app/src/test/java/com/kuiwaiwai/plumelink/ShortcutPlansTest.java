package com.kuiwaiwai.plumelink;

import org.json.*;
import org.junit.Test;
import static org.junit.Assert.*;

public class ShortcutPlansTest {
    static JSONObject plan(String id, String name) throws Exception {
        return new JSONObject().put("id", "p" + id.repeat(32)).put("name", name)
            .put("shortcuts", new JSONObject(MobilePreferencesTest.sample()).getJSONArray("shortcuts"));
    }
    @Test public void preservesUnicodeNamesAndCustomizedIconsInTheCompleteSnapshot() throws Exception {
        JSONArray plans = new JSONArray().put(plan("a", "工作 😀"));
        String saved = MobilePreferences.incoming(new JSONObject(MobilePreferencesTest.sample()).put("shortcutPlans", plans).toString());
        JSONObject restored = new JSONObject(saved).getJSONArray("shortcutPlans").getJSONObject(0);
        assertEquals("工作 😀", restored.getString("name"));
        assertEquals("heart", restored.getJSONArray("shortcuts").getJSONObject(0).getString("icon"));
        assertFalse(ShortcutPlans.normalize(new JSONArray().put(plan("b", "😀".repeat(32)))).isEmpty());
    }
    @Test public void rejectsDuplicateIdsNamesActionsAndUnknownFields() throws Exception {
        assertEquals("", ShortcutPlans.normalize(new JSONArray().put(plan("a", "Work")).put(plan("a", "Other"))));
        assertEquals("", ShortcutPlans.normalize(new JSONArray().put(plan("a", "Work")).put(plan("b", "WORK"))));
        assertEquals("", ShortcutPlans.normalize(new JSONArray().put(plan("a", "Work").put("token", "private"))));
        JSONObject duplicate = plan("a", "Work");
        duplicate.getJSONArray("shortcuts").put(1, duplicate.getJSONArray("shortcuts").getJSONObject(0));
        assertEquals("", ShortcutPlans.normalize(new JSONArray().put(duplicate)));
    }
    @Test public void rejectsMalformedOversizedAndUnsafeNames() throws Exception {
        for (String name : new String[]{"", " Work", "Work ", "x".repeat(33), "line\nname", "bad\u202ename", "e\u0301"})
            assertEquals(name, "", ShortcutPlans.normalize(new JSONArray().put(plan("a", name))));
        JSONArray many = new JSONArray();
        for (int i=0;i<9;i++) many.put(plan(Integer.toHexString(i), "Plan " + i));
        assertEquals("", ShortcutPlans.normalize(many));
        assertEquals("", ShortcutPlans.normalize("[]"));
        assertEquals("", ShortcutPlans.normalize(null));
        assertEquals("[]", ShortcutPlans.normalize(new JSONArray()));
    }
}
