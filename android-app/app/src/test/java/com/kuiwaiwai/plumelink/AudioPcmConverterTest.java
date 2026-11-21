package com.kuiwaiwai.plumelink;

import java.util.Arrays;
import java.util.Random;
import org.junit.Test;
import static org.junit.Assert.*;

public class AudioPcmConverterTest {
    @Test public void fallbackPreservesPcmExactly() {
        short[] input = {Short.MIN_VALUE, -1, 0, 1, Short.MAX_VALUE};
        short[] output = new short[input.length];
        assertEquals(input.length, new AudioPcmConverter(24000).convert(input, 0, input.length, output));
        assertArrayEquals(input, output);
    }

    @Test public void readBoundariesDoNotChangeSamplesOrLoseTheDecimationPhase() {
        short[] input = new short[48001]; Random random = new Random(7);
        for (int i = 0; i < input.length; i++) input[i] = (short)random.nextInt();
        short[] expected = new short[24000], actual = new short[24000], chunk = new short[960];
        assertEquals(expected.length, new AudioPcmConverter(48000).convert(input, 0, input.length, expected));
        AudioPcmConverter converter = new AudioPcmConverter(48000);
        int written = 0;
        for (int offset = 0; offset < input.length;) {
            int count = Math.min(1 + random.nextInt(960), input.length - offset);
            int n = converter.convert(input, offset, count, chunk);
            System.arraycopy(chunk, 0, actual, written, n);
            offset += count; written += n;
        }
        assertEquals(expected.length, written);
        assertArrayEquals(expected, actual);
    }

    @Test public void speechBandPassesAndUltrasonicEnergyDoesNotAliasIntoVoice() {
        for (int hz : new int[]{300, 1000, 4000, 8000}) assertEquals("passband " + hz, 1.0, gain(hz), 0.015);
        for (int hz : new int[]{12000, 14000, 18000, 23000}) assertTrue("stopband " + hz, gain(hz) < 0.0032);
    }

    @Test public void silenceHasNoAddedNoiseAndFullScaleDoesNotWrap() {
        short[] output = new short[480];
        AudioPcmConverter converter = new AudioPcmConverter(48000);
        converter.convert(new short[960], 0, 960, output);
        assertArrayEquals(new short[480], output);
        for (short value : new short[]{Short.MIN_VALUE, Short.MAX_VALUE}) {
            converter = new AudioPcmConverter(48000);
            short[] input = new short[960]; Arrays.fill(input, value);
            converter.convert(input, 0, input.length, output);
            for (int i = 0; i < output.length; i++) {
                if (i > 50) assertEquals(value, output[i]);
                // A negative full-scale step must never wrap around to a large positive value (or vice versa).
                assertTrue(Math.signum(value) * output[i] > -10000);
            }
        }
    }

    @Test public void rejectedOutputCapacityDoesNotAdvanceFilterHistory() {
        short[] input = new short[960]; Arrays.fill(input, (short)2000);
        AudioPcmConverter converter = new AudioPcmConverter(48000);
        assertThrows(IllegalArgumentException.class, () -> converter.convert(input, 0, 960, new short[479]));
        short[] expected = new short[480], actual = new short[480];
        converter.convert(input, 0, 960, actual);
        new AudioPcmConverter(48000).convert(input, 0, 960, expected);
        assertArrayEquals(expected, actual);
    }

    private static double gain(int hz) {
        short[] input = new short[48000], output = new short[24000];
        for (int i = 0; i < input.length; i++) input[i] = (short)Math.round(10000 * Math.sin(2 * Math.PI * hz * i / 48000));
        new AudioPcmConverter(48000).convert(input, 0, input.length, output);
        double energy = 0;
        for (int i = 480; i < output.length; i++) energy += (double)output[i] * output[i];
        return Math.sqrt(energy / (output.length - 480)) / (10000 / Math.sqrt(2));
    }
}
