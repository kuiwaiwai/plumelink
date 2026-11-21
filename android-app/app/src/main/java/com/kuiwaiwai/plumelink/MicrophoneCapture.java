package com.kuiwaiwai.plumelink;

import android.annotation.SuppressLint;
import android.content.Context;
import android.media.*;
import android.os.Build;
import android.os.SystemClock;
import android.util.Log;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Arrays;
import java.util.concurrent.atomic.AtomicBoolean;
import okhttp3.*;
import okio.ByteString;
import org.json.JSONObject;

/** Service-owned, bounded-latency capture. Each instance owns one transport attempt. */
final class MicrophoneCapture extends WebSocketListener {
    interface Events { void state(String phase, double level, String message); }
    private final Events events;
    private final AudioManager audioManager;
    private final Context appContext;
    private final AtomicBoolean stopped = new AtomicBoolean();
    private final AtomicBoolean recording = new AtomicBoolean();
    private final Object recorderLock = new Object();
    private final Object watchdogLock = new Object();
    private volatile AudioRecord recorder;
    private volatile WebSocket socket;
    private volatile long captureProgressAt = -1;
    private final AudioSendWindow window = new AudioSendWindow();
    private final android.os.Handler watchdog = new android.os.Handler(android.os.Looper.getMainLooper());
    private final Runnable checkAcknowledgements = new Runnable() {
        @Override public void run() {
            if (stopped.get()) return;
            long now = SystemClock.elapsedRealtime();
            if (captureProgressAt >= 0 && now - captureProgressAt >= 1200) fail("手机麦克风持续未返回声音数据，请检查是否被其他应用占用后重新开启");
            else if (window.expired(now)) networkFailure();
            else synchronized (watchdogLock) { if (!stopped.get()) watchdog.postDelayed(this, 100); }
        }
    };
    private String serverSession = "";
    private boolean confirmed;
    private volatile OkHttpClient client;
    private volatile AudioFocusRequest focusRequest;
    private final String token;
    private final AudioManager.OnAudioFocusChangeListener focusListener = change -> {
        if (change == AudioManager.AUDIOFOCUS_LOSS || change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT) {
            if (!stopped.get()) fail("音频焦点被其他应用或通话占用，已停止麦克风");
        }
    };
    private static String detail(Exception error, String fallback) {
        String message = error == null ? null : error.getMessage();
        return message == null || message.isEmpty() ? fallback : message;
    }
    MicrophoneCapture(Context context, String base, String host, String pin, String token, String resume, Events events) {
        this.events = events; this.token = token;
        appContext = context.getApplicationContext();
        audioManager = context.getSystemService(AudioManager.class);
        new Thread(() -> {
            try {
                client = new PinnedConnections(host, pin).audioClient();
                if (stopped.get()) { disposeClient(); return; }
                Request request = new Request.Builder().url(base.replaceFirst("^https:", "wss:") + "/microphone" + (resume.isEmpty() ? "" : "?resume=" + resume)).header("Origin", base).build();
                socket = client.newWebSocket(request, this);
                if (stopped.get()) { socket.cancel(); return; }
                synchronized (watchdogLock) {
                    if (!stopped.get()) watchdog.postDelayed(() -> { if (!recording.get() && !stopped.get()) networkFailure(); }, 5000);
                }
            } catch (Exception e) { fail("无法连接电脑音频：" + detail(e, "系统未返回具体原因")); }
        }, "PlumeLink-microphone-connect").start();
    }
    @Override public void onOpen(WebSocket ws, Response response) {
        if (stopped.get()) { ws.cancel(); return; }
        try { if (!ws.send(new JSONObject().put("kind", "auth").put("token", token).toString())) fail("麦克风认证连接已关闭"); }
        catch (Exception e) { fail("麦克风认证失败"); }
    }
    @Override public void onMessage(WebSocket ws, String message) {
        if (stopped.get()) return;
        try {
            JSONObject frame = new JSONObject(message);
            String kind = frame.getString("kind");
            if ("ready".equals(kind) && frame.getInt("rate") == 24000 && frame.getInt("samples") == 480 && frame.getInt("window") == 4) {
                if (!recording.compareAndSet(false, true)) throw new Exception("重复初始化");
                serverSession = frame.optString("session", "");
                captureProgressAt = SystemClock.elapsedRealtime();
                synchronized (watchdogLock) { if (!stopped.get()) watchdog.post(checkAcknowledgements); }
                new Thread(() -> capture(ws), "PlumeLink-microphone-capture").start();
            } else if ("ack".equals(kind)) {
                int ack = frame.getInt("sequence");
                if (!window.acknowledge(ack)) throw new Exception("音频确认无效");
                if (!confirmed && serverSession.matches("[0-9]{1,20}")) {
                    confirmed = true;
                    events.state("confirmed", 0, serverSession);
                }
            } else if ("error".equals(kind)) fail(frame.optString("message", "电脑音频不可用"));
            else throw new Exception("音频配置不匹配");
        } catch (Exception e) { fail(detail(e, "电脑音频响应无效")); }
    }
    private final class CaptureDevice implements AudioCaptureSetup.Device {
        final AudioRecord audio;
        final int rate;
        CaptureDevice(AudioRecord audio, int rate) { this.audio = audio; this.rate = rate; }
        public boolean initialized() { return audio.getState() == AudioRecord.STATE_INITIALIZED && audio.getSampleRate() == rate; }
        @SuppressLint("MissingPermission") // The visible Activity grants permission and the service establishes its microphone type.
        public void start() { audio.startRecording(); }
        public boolean recording() { return audio.getRecordingState() == AudioRecord.RECORDSTATE_RECORDING; }
        public void close() {
            synchronized (recorderLock) {
                if (recorder == audio) recorder = null;
                try { audio.stop(); } catch (IllegalStateException ignored) {}
                audio.release();
            }
        }
    }

    @SuppressLint("MissingPermission")
    private CaptureDevice createDevice(int rate, AudioCaptureSetup.Source source) {
        int size = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT);
        if (size <= 0) throw new IllegalArgumentException("不支持采样率 " + rate);
        int audioSource = source == AudioCaptureSetup.Source.VOICE_RECOGNITION
            ? MediaRecorder.AudioSource.VOICE_RECOGNITION : MediaRecorder.AudioSource.MIC;
        AudioRecord audio = new AudioRecord(audioSource, rate, AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT, Math.max(size, rate * 2 * 80 / 1000));
        preferExternalInputDevice(audio);
        synchronized (recorderLock) {
            if (stopped.get()) {
                audio.release();
                throw new IllegalStateException("麦克风已停止");
            }
            recorder = audio;
        }
        return new CaptureDevice(audio, rate);
    }

    private String captureWarning(AudioRecord audio) {
        if (audioManager != null && audioManager.isMicrophoneMute()) return AppLanguage.text(appContext, R.string.mic_privacy_muted);
        if (Build.VERSION.SDK_INT >= 29) {
            AudioRecordingConfiguration configuration = audio.getActiveRecordingConfiguration();
            if (configuration != null && configuration.isClientSilenced()) return AppLanguage.text(appContext, R.string.mic_capture_silenced);
        }
        return "";
    }

    private void preferExternalInputDevice(AudioRecord audio) {
        if (audioManager == null) return;
        AudioDeviceInfo chosen = null;
        int bestRank = Integer.MAX_VALUE;
        for (AudioDeviceInfo info : audioManager.getDevices(AudioManager.GET_DEVICES_INPUTS)) {
            int type = info.getType();
            int rank = type == AudioDeviceInfo.TYPE_USB_HEADSET ? 1
                : type == AudioDeviceInfo.TYPE_WIRED_HEADSET ? 2
                : (Build.VERSION.SDK_INT >= 31 && type == AudioDeviceInfo.TYPE_BLE_HEADSET) ? 3
                : type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO ? 4
                : Integer.MAX_VALUE;
            if (rank < bestRank) {
                bestRank = rank;
                chosen = info;
            }
        }
        if (chosen != null) {
            try { audio.setPreferredDevice(chosen); } catch (Exception ignored) {}
        }
    }

    private void acquireAudioFocus() {
        if (audioManager == null) return;
        AudioFocusRequest req = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE)
            .setAudioAttributes(new AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                .build())
            .setOnAudioFocusChangeListener(focusListener)
            .build();
        focusRequest = req;
        try { audioManager.requestAudioFocus(req); } catch (Exception ignored) {}
    }

    private void releaseAudioFocus() {
        AudioFocusRequest req = focusRequest;
        focusRequest = null;
        if (audioManager != null && req != null) {
            try { audioManager.abandonAudioFocusRequest(req); } catch (Exception ignored) {}
        }
    }

    private void capture(WebSocket ws) {
        CaptureDevice device = null;
        android.media.audiofx.NoiseSuppressor suppressor = null;
        try {
            android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO);
            acquireAudioFocus();
            var opened = AudioCaptureSetup.open(this::createDevice, stopped::get);
            device = opened.device;
            AudioRecord audio = device.audio;
            int captureRate = opened.rate;
            Log.i("PlumeLink-microphone", "Capture " + captureRate + " Hz / " + opened.source + "; wire 24000 Hz mono PCM16");
            if (stopped.get()) return;
            if (opened.source == AudioCaptureSetup.Source.MICROPHONE
                && android.media.audiofx.NoiseSuppressor.isAvailable()) {
                try { suppressor = android.media.audiofx.NoiseSuppressor.create(audio.getAudioSessionId());
                    if (suppressor != null) suppressor.setEnabled(true);
                } catch (RuntimeException ignored) { /* Capture remains usable without device DSP. */ }
            }
            captureProgressAt = SystemClock.elapsedRealtime();
            // Report live only after the first complete read, including an explicit system-silenced state.
            AudioPcmConverter converter = new AudioPcmConverter(captureRate);
            short[] captured = new short[captureRate / 50];
            short[] samples = new short[AudioPcmConverter.FRAME_SAMPLES];
            byte[] frame = new byte[972]; ByteBuffer bytes = ByteBuffer.wrap(frame).order(ByteOrder.LITTLE_ENDIAN);
            long meterAt = 0, capturedSamples = 0;
            String warning = "";
            AudioTimestamp timestamp = new AudioTimestamp();
            long hardwarePosition = -1, hardwareNanos = 0;
            int filled = 0;
            while (!stopped.get()) {
                int count = audio.read(captured, filled, captured.length - filled, AudioRecord.READ_BLOCKING);
                if (stopped.get()) break;
                if (count <= 0) {
                    if (count == AudioRecord.ERROR_DEAD_OBJECT) throw new Exception("手机录音设备已断开，请重新开启麦克风");
                    throw new Exception("手机收音失败（" + count + "），请检查权限或其他应用占用");
                }
                filled += count; if (filled < captured.length) continue; filled = 0;
                capturedSamples += captured.length;
                long now = SystemClock.elapsedRealtime();
                captureProgressAt = now;
                if (converter.convert(captured, 0, captured.length, samples) != samples.length) throw new Exception("音频采样转换失败");
                if (window.expired(now)) { networkFailure(); return; }
                boolean timestampUpdated = false;
                if (audio.getTimestamp(timestamp, AudioTimestamp.TIMEBASE_MONOTONIC) == AudioRecord.SUCCESS
                    && timestamp.nanoTime > 0 && timestamp.nanoTime != hardwareNanos) {
                    hardwarePosition = timestamp.framePosition;
                    hardwareNanos = timestamp.nanoTime;
                    timestampUpdated = true;
                }
                // Samples waiting in the recorder must not be sent as fresh after a scheduling stall.
                if (timestampUpdated && hardwarePosition >= capturedSamples) {
                    long ahead = hardwarePosition - capturedSamples;
                    if (ahead > captureRate / 20L) {
                        // Resync after HAL ring-buffer overruns so future frames are not permanently aged out.
                        capturedSamples = hardwarePosition;
                        ahead = 0;
                    }
                    long capturedAt = hardwareNanos - ahead * 1_000_000_000L / captureRate;
                    if (System.nanoTime() - capturedAt > 120_000_000L) continue;
                }
                boolean updateMeter = now - meterAt >= 100;
                if (updateMeter) warning = captureWarning(audio);
                if (!warning.isEmpty()) Arrays.fill(samples, (short)0);
                if (stopped.get()) break;
                if (updateMeter) {
                    meterAt = now;
                    double energy = 0; for (short sample : samples) energy += (double)sample * sample;
                    double level = Math.min(1, Math.sqrt(energy / samples.length) / 12000.0);
                    events.state("live", level, warning);
                }
                // Skip fresh samples when the receiver is behind; never replay a capture backlog.
                int sequence = window.reserve(now, ws.queueSize());
                if (sequence == 0) continue;
                long capturedMs = capturedSamples * 1000 / captureRate;
                if (capturedMs >= Integer.MAX_VALUE) throw new Exception("音频会话已到期，请重新开启");
                bytes.clear(); bytes.putInt(0x314d4651); bytes.putInt(sequence); bytes.putInt((int)capturedMs);
                for (short sample : samples) bytes.putShort(sample);
                if (!ws.send(ByteString.of(frame))) { networkFailure(); return; }
            }
        } catch (SecurityException e) { if (!stopped.get()) fail("手机录音权限不可用，请允许录音后重新开启麦克风"); }
        catch (Exception e) {
            if (!stopped.get()) { Log.w("PlumeLink-microphone", "Capture failed", e); fail(detail(e, "手机麦克风读取失败")); }
        }
        finally {
            captureProgressAt = -1;
            releaseAudioFocus();
            try { if (suppressor != null) suppressor.release(); }
            finally { if (device != null) device.close(); }
        }
    }
    @Override public void onFailure(WebSocket ws, Throwable t, Response response) {
        if (certificateRejected(t)) fail("无法验证电脑证书，请重新核对电脑身份");
        else if (response != null) fail("电脑拒绝音频连接，请检查麦克风权限");
        else networkFailure();
    }
    @Override public void onClosing(WebSocket ws, int code, String reason) {
        if (code == 4008) networkFailure();
        else fail(code == 4003 ? "电脑拒绝音频连接，请检查麦克风权限" : "电脑已停止接收麦克风");
    }
    private static boolean certificateRejected(Throwable cause) {
        for (Throwable current = cause; current != null; current = current.getCause()) {
            if (current instanceof javax.net.ssl.SSLPeerUnverifiedException
                    || current instanceof java.security.cert.CertificateException) return true;
        }
        return false;
    }
    private void networkFailure() {
        if (stop()) events.state("networkError", 0, "电脑音频连接无响应，请检查局域网连接");
    }
    private void fail(String message) {
        if (!stop()) return;
        String safe = message == null ? "" : message.trim();
        events.state("error", 0, safe.isEmpty() ? "麦克风已停止" : safe);
    }
    boolean stop() {
        if (!stopped.compareAndSet(false, true)) return false;
        synchronized (watchdogLock) { watchdog.removeCallbacksAndMessages(null); }
        WebSocket ws = socket; if (ws != null) ws.cancel();
        synchronized (recorderLock) {
            AudioRecord audio = recorder;
            if (audio != null) try { audio.stop(); } catch (Exception ignored) {}
        }
        disposeClient(); return true;
    }
    private void disposeClient() { OkHttpClient current = client; if (current != null) { current.connectionPool().evictAll(); current.dispatcher().executorService().shutdown(); } }
}
