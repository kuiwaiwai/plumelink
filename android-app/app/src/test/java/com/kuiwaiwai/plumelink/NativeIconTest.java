package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.assertNotNull;

public class NativeIconTest {
    @Test public void everyShortcutIconParsesWithAndroidPathParser() {
        String[] icons = {
            "star", "bolt", "bell", "grid", "monitor", "pointer", "volume", "search",
            "switch", "left", "right", "layers", "keyboard", "command", "mic", "gamepad",
            "settings", "activity", "heart", "idea", "play", "previous", "nexttrack", "stop", "alert-circle", "check-circle"
        };
        for (String icon : icons) assertNotNull(icon, NativeIcon.parse(icon));
    }
}
