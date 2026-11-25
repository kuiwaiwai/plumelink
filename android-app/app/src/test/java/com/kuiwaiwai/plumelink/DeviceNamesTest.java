package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;

public class DeviceNamesTest {
    @Test public void prefersTheSystemNameAndFallsBackWithoutPermissions() {
        assertEquals("我的手机", DeviceNames.resolve(() -> " 我的手机 ", () -> "other", "2206122SC", "Android"));
        assertEquals("Xiaomi 12S Pro", DeviceNames.resolve(() -> { throw new SecurityException(); }, () -> "Xiaomi 12S Pro", "2206122SC", "Android"));
        assertEquals("2206122SC", DeviceNames.resolve(() -> null, () -> "null", "2206122SC", "Android"));
        assertEquals("Android", DeviceNames.resolve(() -> "", () -> "unknown", null, "Android"));
    }

    @Test public void boundsUnicodeNamesWithoutSplittingCharacters() {
        assertEquals("我的手机", DeviceNames.clean("\n我的\t手机\r"));
        String name = String.join("", java.util.Collections.nCopies(50, "📱"));
        String result = DeviceNames.clean(name);
        assertEquals(40, result.codePointCount(0, result.length()));
        assertFalse(Character.isHighSurrogate(result.charAt(result.length() - 1)));
    }
}
