package com.kuiwaiwai.plumelink;

import java.util.ArrayList;
import java.util.List;
import org.junit.Test;
import static org.junit.Assert.*;

public class HardwareVolumeKeysTest {
    private final List<String> events = new ArrayList<>();
    private final HardwareVolumeKeys keys = new HardwareVolumeKeys((id, down) -> events.add(id + ":" + down));

    @Test public void disabledKeysAndOrphanRepeatsFallThrough() {
        assertFalse(keys.down(24, false, ""));
        assertFalse(keys.up(24));
        assertFalse(keys.down(24, true, "up"));
        assertTrue(events.isEmpty());
    }
    @Test public void repeatsAndReleaseUseOnlyTheInitialBinding() {
        assertTrue(keys.down(24, false, "up:screen:1"));
        for (int i = 0; i < 20; i++) assertTrue(keys.down(24, true, "up:screen:2"));
        assertTrue(keys.up(24));
        assertEquals(List.of("up:screen:1:true", "up:screen:1:false"), events);
        assertFalse(keys.up(24));
    }
    @Test public void cancellationReleasesOnceAndConsumesTheRestOfTheHold() {
        keys.down(24, false, "up"); keys.cancel(); keys.cancel();
        assertTrue(keys.down(24, true, "up"));
        assertTrue(keys.up(24));
        assertEquals(List.of("up:true", "up:false"), events);
    }
    @Test public void newPressAfterLostReleaseCanUseANewBinding() {
        keys.down(24, false, "old"); keys.cancel();
        assertTrue(keys.down(24, false, "new")); keys.up(24);
        assertEquals(List.of("old:true", "old:false", "new:true", "new:false"), events);
    }
    @Test public void resetReleasesBothKeysAndDoesNotCarryThemToTheNextPage() {
        keys.down(24, false, "up"); keys.down(25, false, "down"); keys.reset();
        assertTrue(events.containsAll(List.of("up:true", "down:true", "up:false", "down:false")));
        assertEquals(4, events.size());
        assertFalse(keys.up(24)); assertFalse(keys.up(25));
        assertTrue(keys.down(25, false, "new")); keys.up(25);
        assertEquals(List.of("new:true", "new:false"), events.subList(4, 6));
    }
    @Test public void releasingOneKeyDoesNotReleaseTheOther() {
        keys.down(24, false, "up"); keys.down(25, false, "down"); keys.up(24);
        assertTrue(keys.down(25, true, "down"));
        assertEquals(List.of("up:true", "down:true", "up:false"), events);
        keys.up(25); assertEquals("down:false", events.get(3));
    }
}
