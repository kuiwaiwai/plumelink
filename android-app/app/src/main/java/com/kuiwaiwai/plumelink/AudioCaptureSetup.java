package com.kuiwaiwai.plumelink;

import java.util.concurrent.CancellationException;
import java.util.function.BooleanSupplier;

/** Owns each candidate until a recording device is successfully handed to the session. */
final class AudioCaptureSetup {
    enum Source { VOICE_RECOGNITION, MICROPHONE }
    interface Device extends AutoCloseable {
        boolean initialized();
        void start();
        boolean recording();
        @Override void close();
    }
    interface Factory<T extends Device> { T create(int rate, Source source); }
    static final class Opened<T extends Device> {
        final T device;
        final int rate;
        final Source source;
        Opened(T device, int rate, Source source) {
            this.device = device; this.rate = rate; this.source = source;
        }
    }

    static <T extends Device> Opened<T> open(Factory<T> factory, BooleanSupplier cancelled) {
        IllegalStateException failure = new IllegalStateException("手机麦克风初始化失败，请检查录音权限或是否被其他应用占用");
        for (Source source : Source.values()) {
            for (int rate : new int[]{48000, 24000}) {
                checkCancelled(cancelled);
                T device = null;
                boolean transferred = false;
                try {
                    device = factory.create(rate, source);
                    checkCancelled(cancelled);
                    if (!device.initialized()) throw new IllegalStateException("录音格式不可用");
                    device.start();
                    checkCancelled(cancelled);
                    if (!device.recording()) throw new IllegalStateException("无法开始收音");
                    transferred = true;
                    return new Opened<>(device, rate, source);
                } catch (CancellationException e) {
                    throw e;
                } catch (IllegalArgumentException | IllegalStateException e) {
                    failure.addSuppressed(e);
                } finally {
                    if (device != null && !transferred) device.close();
                }
            }
        }
        throw failure;
    }

    private static void checkCancelled(BooleanSupplier cancelled) {
        if (cancelled.getAsBoolean()) throw new CancellationException("麦克风启动已取消");
    }
}
