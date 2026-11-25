package com.kuiwaiwai.plumelink;

import java.util.Arrays;
import java.util.Random;
import java.security.MessageDigest;
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
        return gain(48000, hz);
    }

    private static double gain(int rate, int hz) {
        short[] input = new short[rate], output = new short[24000];
        for (int i = 0; i < input.length; i++) input[i] = (short)Math.round(10000 * Math.sin(2 * Math.PI * hz * i / rate));
        new AudioPcmConverter(rate).convert(input, 0, input.length, output);
        double energy = 0;
        for (int i = 480; i < output.length; i++) energy += (double)output[i] * output[i];
        return Math.sqrt(energy / (output.length - 480)) / (10000 / Math.sqrt(2));
    }

    @Test public void existing48kOutputMatchesThePreFallbackPcmDigest() throws Exception {
        short[] input = new short[48001]; Random random = new Random(7);
        for (int i = 0; i < input.length; i++) input[i] = (short)random.nextInt();
        short[] output = new short[24000];
        assertEquals(24000, new AudioPcmConverter(48000).convert(input, 0, input.length, output));
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        for (short sample : output) { digest.update((byte)sample); digest.update((byte)(sample >> 8)); }
        StringBuilder hex = new StringBuilder();
        for (byte b : digest.digest()) hex.append(String.format("%02x", b & 255));
        assertEquals("9e27fbe87d94b3fcac62285c4c608c441459528ec44e0d4af2e64b711706f29e", hex.toString());
    }

    @Test public void guaranteedRateProducesExactly480SamplesPer20msWithoutDrift() {
        AudioPcmConverter converter = new AudioPcmConverter(44100);
        short[] input = new short[882], output = new short[480];
        int total = 0;
        for (int frame = 0; frame < 3000; frame++) {
            int count = converter.convert(input, 0, input.length, output);
            assertEquals(480, count);
            total += count;
        }
        assertEquals(60 * 24000, total);
        assertArrayEquals(new short[480], output);
    }

    @Test public void guaranteedRateKeepsPhaseAndHistoryAcrossArbitraryReadBoundaries() {
        short[] input = new short[44107]; Random random = new Random(73);
        for (int i = 0; i < input.length; i++) input[i] = (short)random.nextInt();
        short[] expected = new short[input.length * 80 / 147], actual = new short[expected.length];
        assertEquals(expected.length, new AudioPcmConverter(44100).convert(input, 0, input.length, expected));
        AudioPcmConverter converter = new AudioPcmConverter(44100);
        short[] chunk = new short[481];
        int written = 0;
        for (int offset = 0; offset < input.length;) {
            assertEquals(0, converter.convert(input, offset, 0, new short[0]));
            int count = Math.min(1 + random.nextInt(882), input.length - offset);
            int n = converter.convert(input, offset, count, chunk);
            System.arraycopy(chunk, 0, actual, written, n);
            offset += count; written += n;
        }
        assertEquals(expected.length, written);
        assertArrayEquals(expected, actual);
    }

    @Test public void guaranteedRatePreservesSpeechAndRejectsAboveNyquistEnergy() {
        for (int hz : new int[]{300, 1000, 4000, 8000}) assertEquals("44.1k passband " + hz, 1.0, gain(44100, hz), 0.015);
        for (int hz : new int[]{12000, 14000, 18000, 21900}) assertTrue("44.1k stopband " + hz, gain(44100, hz) < 0.0032);
    }

    @Test public void guaranteedRateHasUnitDcGainForEveryPhaseAndDoesNotWrap() {
        short[] output = new short[480];
        for (short value : new short[]{Short.MIN_VALUE, -7000, 7000, Short.MAX_VALUE}) {
            short[] input = new short[882]; Arrays.fill(input, value);
            new AudioPcmConverter(44100).convert(input, 0, input.length, output);
            for (int i = 0; i < output.length; i++) {
                if (i > 60) assertEquals(value, output[i]);
                assertTrue(Math.signum(value) * output[i] > -10000);
            }
        }
    }

    @Test public void guaranteedRateRejectsInvalidCallsBeforeAdvancingState() {
        short[] input = new short[883]; Arrays.fill(input, (short)2000);
        AudioPcmConverter converter = new AudioPcmConverter(44100), reference = new AudioPcmConverter(44100);
        assertEquals(0, converter.convert(input, 0, 1, new short[0]));
        assertEquals(0, reference.convert(input, 0, 1, new short[0]));
        assertThrows(IllegalArgumentException.class, () -> converter.convert(input, 1, 882, new short[479]));
        assertThrows(IllegalArgumentException.class, () -> converter.convert(input, -1, 882, new short[480]));
        assertThrows(IllegalArgumentException.class, () -> converter.convert(input, 2, 882, new short[480]));
        assertThrows(IllegalArgumentException.class, () -> converter.convert(input, 0, -1, new short[480]));
        short[] expected = new short[480], actual = new short[480];
        assertEquals(480, reference.convert(input, 1, 882, expected));
        assertEquals(480, converter.convert(input, 1, 882, actual));
        assertArrayEquals(expected, actual);
        assertThrows(IllegalArgumentException.class, () -> new AudioPcmConverter(16000));
        assertThrows(IllegalArgumentException.class, () -> new AudioPcmConverter(0));
    }

    @Test public void guaranteedRateImpulseHasTheFixedFilterDelay() {
        short[] input = new short[882], output = new short[480];
        input[147] = 20000;
        new AudioPcmConverter(44100).convert(input, 0, input.length, output);
        double energy = 0, weightedTime = 0;
        for (int i = 0; i < output.length; i++) {
            double square = (double)output[i] * output[i];
            energy += square;
            weightedTime += square * ((i + 1) * 147.0 / 80 - 1);
        }
        assertTrue(energy > 0);
        assertEquals(147 + 47, weightedTime / energy, 147.0 / 80);
    }
}
