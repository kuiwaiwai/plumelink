package com.kuiwaiwai.plumelink;

/** Streaming 48 -> 24 kHz conversion; filter history survives every read boundary. */
final class AudioPcmConverter {
    static final int OUTPUT_RATE = 24000;
    static final int FRAME_SAMPLES = 480;
    private static final int TAPS = 95;
    private static final int HALF = (TAPS - 1) / 2;
    private static final double[] FOLDED = lowPassHalf();
    private static final double CENTER_WEIGHT = FOLDED[0];
    private final int rate;
    // Mirrored ring buffer removes boundary checks from the symmetric MAC inner loop.
    private final short[] history = new short[TAPS * 2];
    private int position, phase;

    AudioPcmConverter(int rate) {
        if (rate != OUTPUT_RATE && rate != OUTPUT_RATE * 2) throw new IllegalArgumentException("Unsupported capture rate");
        this.rate = rate;
    }

    int convert(short[] input, int offset, int count, short[] output) {
        if (offset < 0 || count < 0 || offset > input.length - count) throw new IllegalArgumentException("Invalid input range");
        int needed = rate == OUTPUT_RATE ? count : (count + phase) / 2;
        if (output.length < needed) throw new IllegalArgumentException("Output buffer too small");
        if (rate == OUTPUT_RATE) {
            System.arraycopy(input, offset, output, 0, count);
            return count;
        }
        int written = 0;
        for (int i = offset; i < offset + count; i++) {
            short v = input[i];
            history[position] = v;
            history[position + TAPS] = v;
            if (++position == TAPS) position = 0;
            if (++phase < 2) continue;
            phase = 0;
            int center = position + HALF;
            double sample = CENTER_WEIGHT * history[center];
            for (int d = 1; d <= HALF; d++) {
                sample += FOLDED[d] * (history[center - d] + history[center + d]);
            }
            output[written++] = (short)Math.max(Short.MIN_VALUE, Math.min(Short.MAX_VALUE, Math.round(sample)));
        }
        return written;
    }

    private static double[] lowPassHalf() {
        double[] coefficients = new double[TAPS];
        // Leave a transition band below the 12 kHz output Nyquist limit. Delay: 47 / 48000 s.
        double cutoff = 10000.0 / 48000.0, sum = 0;
        for (int i = 0; i < TAPS; i++) {
            int distance = i - HALF;
            double sinc = distance == 0 ? 2 * cutoff : Math.sin(2 * Math.PI * cutoff * distance) / (Math.PI * distance);
            double window = 0.42 - 0.5 * Math.cos(2 * Math.PI * i / (TAPS - 1))
                + 0.08 * Math.cos(4 * Math.PI * i / (TAPS - 1));
            coefficients[i] = sinc * window;
            sum += coefficients[i];
        }
        double[] half = new double[HALF + 1];
        for (int d = 0; d <= HALF; d++) half[d] = coefficients[HALF - d] / sum;
        return half;
    }
}
