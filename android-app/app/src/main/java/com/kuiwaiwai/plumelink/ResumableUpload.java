package com.kuiwaiwai.plumelink;

import android.content.ContentResolver;
import android.database.Cursor;
import android.net.Uri;
import android.os.SystemClock;
import android.provider.OpenableColumns;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.HashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.atomic.*;
import java.util.concurrent.*;
import java.util.function.BooleanSupplier;
import javax.net.ssl.HttpsURLConnection;
import okhttp3.Call;
import okhttp3.MediaType;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.RequestBody;
import okhttp3.Response;
import okhttp3.ResponseBody;
import okio.BufferedSink;
import org.json.JSONObject;

/** A single bounded chunk remains available until its server-confirmed offset advances. */
final class ResumableUpload {
    interface Progress { void update(String name, int percent, boolean retrying); }
    interface Connections { HttpsURLConnection open(String url, String token) throws Exception; }
    private static final class ResponseData {
        final int status;
        final String body;
        ResponseData(int status, String body) { this.status = status; this.body = body; }
    }
    private interface Transport {
        ResponseData send(String path, String method, Map<String, String> headers, byte[] bytes, int count) throws Exception;
        void deleteAsync(String path);
        void close();
    }
    private static final int CHUNK = 4 * 1024 * 1024;
    private static final class Permanent extends TransferFailure {
        Permanent(int reason, Object... arguments) { super(reason, arguments); }
    }
    private final Transport transport;
    private final String base, token;
    private final AtomicBoolean cancelled;
    private final long deadline = SystemClock.elapsedRealtime() + 2 * 60 * 60 * 1000L;
    private ResumableUpload(Transport transport, String base, String token, AtomicBoolean cancelled) {
        this.transport = transport; this.base = base; this.token = token; this.cancelled = cancelled;
    }
    private void check() throws Permanent {
        if (cancelled.get() || Thread.currentThread().isInterrupted()) throw new Permanent(R.string.ui_transfer_canceled);
        if (SystemClock.elapsedRealtime() > deadline) throw new Permanent(R.string.ui_transfer_timeout);
    }
    private void pause(int failure) throws Exception { check(); Thread.sleep(Math.min(4000, 500L << failure)); check(); }
    private JSONObject request(String path, String method, byte[] bytes, int count, long offset) throws Exception {
        check();
        Map<String, String> headers = new HashMap<>();
        if (bytes != null && "PUT".equals(method)) {
            MessageDigest sha = MessageDigest.getInstance("SHA-256");
            sha.update(bytes, 0, count);
            StringBuilder hex = new StringBuilder();
            for (byte b : sha.digest()) hex.append(String.format(java.util.Locale.ROOT, "%02x", b));
            headers.put("X-Upload-Offset", Long.toString(offset));
            headers.put("X-Chunk-SHA256", hex.toString());
        }
        ResponseData response = transport.send(path, method, headers, bytes, count);
        int status = response.status;
        if (status != 200 && status != 201) {
            if (status == 400 || status == 401 || status == 403 || status == 404 || status == 413) throw new Permanent(R.string.ui_transfer_upload_rejected, status);
            throw new TransferFailure(R.string.ui_transfer_upload_unavailable, status);
        }
        try {
            return new JSONObject(response.body);
        } catch (Exception error) {
            throw new Permanent(R.string.ui_transfer_invalid_confirmation);
        }
    }
    private static long offset(JSONObject state, long size) throws Exception {
        long value = state.getLong("offset");
        if (state.getLong("size") != size || state.getInt("chunkBytes") != CHUNK || value < 0 || value > size)
            throw new Permanent(R.string.ui_transfer_invalid_confirmation);
        return value;
    }
    static void send(ContentResolver resolver, PinnedConnections network, String base, String token, Uri uri,
            AtomicBoolean cancelled, AtomicReference<Closeable> active, BooleanSupplier realtime, Progress progress) throws Exception {
        String name = null; long size = -1;
        try (Cursor cursor = resolver.query(uri, new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                name = cursor.getString(0);
                if (!cursor.isNull(1)) size = cursor.getLong(1);
            }
        }
        if (size < 0) {
            try (android.content.res.AssetFileDescriptor afd = resolver.openAssetFileDescriptor(uri, "r")) {
                if (afd != null && afd.getLength() >= 0) size = afd.getLength();
            } catch (Exception ignored) {}
        }
        if (size < 0) throw new Permanent(R.string.ui_transfer_unknown_file_size);
        if (!validName(name)) throw new Permanent(R.string.ui_transfer_invalid_file_name);
        if (size > ResumableDownload.MAX_BYTES) throw new Permanent(R.string.ui_transfer_file_too_large);
        try (InputStream input = resolver.openInputStream(uri)) {
            if (input == null) throw new Permanent(R.string.ui_transfer_source_unreadable);
            OkHttpClient client = network.transferClient().newBuilder()
                .readTimeout(90, TimeUnit.SECONDS)
                .writeTimeout(90, TimeUnit.SECONDS)
                .build();
            Transport okTransport = new Transport() {
                @Override public ResponseData send(String path, String method, Map<String, String> headers, byte[] bytes, int count) throws Exception {
                    if (cancelled.get() || Thread.currentThread().isInterrupted()) throw new Permanent(R.string.ui_transfer_canceled);
                    Request.Builder req = new Request.Builder()
                        .url(base + path)
                        .header("Authorization", "Bearer " + token);
                    for (Map.Entry<String, String> entry : headers.entrySet()) req.header(entry.getKey(), entry.getValue());
                    RequestBody body = null;
                    if (bytes != null) {
                        MediaType mediaType = MediaType.get("PUT".equals(method) ? "application/octet-stream" : "application/json");
                        body = new RequestBody() {
                            @Override public MediaType contentType() { return mediaType; }
                            @Override public long contentLength() { return count; }
                            @Override public void writeTo(BufferedSink sink) throws IOException {
                                for (int i = 0; i < count; i += 64 * 1024) {
                                    if (cancelled.get() || Thread.currentThread().isInterrupted()) throw new Permanent(R.string.ui_transfer_canceled);
                                    sink.write(bytes, i, Math.min(64 * 1024, count - i));
                                }
                            }
                        };
                    } else if ("POST".equals(method)) {
                        body = RequestBody.create(new byte[0], null);
                    }
                    req.method(method, body);
                    Call call = client.newCall(req.build());
                    Closeable cancelHandle = call::cancel;
                    active.set(cancelHandle);
                    try (Response res = call.execute()) {
                        ResponseBody responseBody = res.body();
                        String text = "";
                        if (responseBody != null) {
                            byte[] raw = responseBody.bytes();
                            if (raw.length > 16 * 1024) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
                            text = new String(raw, StandardCharsets.UTF_8);
                        }
                        return new ResponseData(res.code(), text);
                    } finally {
                        active.compareAndSet(cancelHandle, null);
                    }
                }
                @Override public void deleteAsync(String path) {
                    Thread cleanup = new Thread(() -> {
                        try {
                            Request req = new Request.Builder().url(base + path).header("Authorization", "Bearer " + token).delete().build();
                            OkHttpClient shortClient = client.newBuilder().connectTimeout(2, TimeUnit.SECONDS).readTimeout(2, TimeUnit.SECONDS).build();
                            try (Response ignored = shortClient.newCall(req).execute()) {}
                        } catch (Exception ignored) {}
                    }, "PlumeLink-upload-delete");
                    cleanup.setDaemon(true);
                    cleanup.start();
                }
                @Override public void close() {}
            };
            sendWithTransport(okTransport, base, token, input, name, size, cancelled, realtime, progress);
        }
    }

    private static boolean windowsDeviceName(String name) {
        String stem = name.split("\\.", 2)[0].trim().toUpperCase(java.util.Locale.ROOT);
        if (java.util.Arrays.asList("CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$").contains(stem)) return true;
        if (stem.length() != 4 || !(stem.startsWith("COM") || stem.startsWith("LPT"))) return false;
        char digit = stem.charAt(3);
        return (digit >= '1' && digit <= '9') || digit == '¹' || digit == '²' || digit == '³';
    }
    private static boolean validName(String name) {
        return name != null && !name.isEmpty() && !name.equals(".") && !name.equals("..") && name.getBytes(StandardCharsets.UTF_8).length <= 240
            && name.indexOf('\\') < 0 && name.indexOf('/') < 0
            && name.chars().noneMatch(ch -> ch < 0x20 || ch == 0x7f || "<>:\"|?*".indexOf(ch) >= 0)
            && !name.endsWith(".") && !name.endsWith(" ") && !name.startsWith(".plumelink-")
            && !windowsDeviceName(name);
    }
    static void sendStream(Connections network, String base, String token, InputStream input, String name, long size,
            AtomicBoolean cancelled, AtomicReference<HttpsURLConnection> active, BooleanSupplier realtime, Progress progress) throws Exception {
        if (!validName(name)) throw new Permanent(R.string.ui_transfer_invalid_file_name);
        if (size < 0) throw new Permanent(R.string.ui_transfer_unknown_file_size);
        if (size > ResumableDownload.MAX_BYTES) throw new Permanent(R.string.ui_transfer_file_too_large);
        ScheduledExecutorService timeouts = Executors.newSingleThreadScheduledExecutor();
        Transport legacyTransport = new Transport() {
            @Override public ResponseData send(String path, String method, Map<String, String> headers, byte[] bytes, int count) throws Exception {
                HttpsURLConnection connection = network.open(base + path, token);
                active.set(connection);
                ScheduledFuture<?> timeout = timeouts.schedule(connection::disconnect, 90, TimeUnit.SECONDS);
                try {
                    if (cancelled.get() || Thread.currentThread().isInterrupted()) throw new Permanent(R.string.ui_transfer_canceled);
                    connection.setReadTimeout(90_000);
                    connection.setRequestMethod(method);
                    if (bytes != null) {
                        connection.setDoOutput(true);
                        connection.setFixedLengthStreamingMode(count);
                        connection.setRequestProperty("Content-Type", "PUT".equals(method) ? "application/octet-stream" : "application/json");
                        for (Map.Entry<String, String> entry : headers.entrySet()) connection.setRequestProperty(entry.getKey(), entry.getValue());
                        try (OutputStream output = connection.getOutputStream()) {
                            for (int i = 0; i < count; i += 64 * 1024) {
                                if (cancelled.get() || Thread.currentThread().isInterrupted()) throw new Permanent(R.string.ui_transfer_canceled);
                                output.write(bytes, i, Math.min(64 * 1024, count - i));
                            }
                        }
                    }
                    int status = connection.getResponseCode();
                    String body = "";
                    if (status == 200 || status == 201) {
                        try (InputStream in = connection.getInputStream(); ByteArrayOutputStream out = new ByteArrayOutputStream()) {
                            byte[] buffer = new byte[2048];
                            int n;
                            while ((n = in.read(buffer)) != -1) {
                                if (cancelled.get() || Thread.currentThread().isInterrupted()) throw new Permanent(R.string.ui_transfer_canceled);
                                if (out.size() + n > 16 * 1024) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
                                out.write(buffer, 0, n);
                            }
                            body = out.toString(StandardCharsets.UTF_8.name());
                        }
                    }
                    return new ResponseData(status, body);
                } finally {
                    timeout.cancel(false);
                    connection.disconnect();
                    active.compareAndSet(connection, null);
                }
            }
            @Override public void deleteAsync(String path) {
                boolean wasInterrupted = Thread.interrupted();
                HttpsURLConnection cleanup = null;
                try { cleanup = network.open(base + path, token); cleanup.setConnectTimeout(3000); cleanup.setReadTimeout(3000); cleanup.setRequestMethod("DELETE"); cleanup.getResponseCode(); }
                catch (Exception ignored) {}
                finally {
                    if (cleanup != null) cleanup.disconnect();
                    if (wasInterrupted) Thread.currentThread().interrupt();
                }
            }
            @Override public void close() { timeouts.shutdownNow(); }
        };
        sendWithTransport(legacyTransport, base, token, input, name, size, cancelled, realtime, progress);
    }
    private static void sendWithTransport(Transport transport, String base, String token, InputStream input, String name, long size,
            AtomicBoolean cancelled, BooleanSupplier realtime, Progress progress) throws Exception {
        if (size < 0) throw new Permanent(R.string.ui_transfer_unknown_file_size);
        if (size > ResumableDownload.MAX_BYTES) throw new Permanent(R.string.ui_transfer_file_too_large);
        ResumableUpload upload = new ResumableUpload(transport, base, token, cancelled);
        String id = UUID.randomUUID().toString().replace("-", ""), path = "/api/uploads/" + id;
        boolean completed = false;
        try {
            byte[] metadata = new JSONObject().put("id", id).put("name", name).put("size", size).toString().getBytes(StandardCharsets.UTF_8);
            JSONObject state;
            for (int attempt = 0; ; attempt++) {
                try { state = upload.request("/api/uploads", "POST", metadata, metadata.length, 0); break; }
                catch (Permanent e) { throw e; }
                catch (Exception e) { if (attempt >= 3) throw e; upload.pause(attempt); }
            }
            if (offset(state, size) != 0 || state.getBoolean("complete")) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
            int initialCap = realtime.getAsBoolean() ? (int) Math.min(size, 64 * 1024) : (int) Math.min(size, CHUNK);
            byte[] chunk = new byte[Math.max(1, initialCap)];
            long confirmed = 0;
            while (confirmed < size) {
                upload.check();
                int count = (int) Math.min(size - confirmed, realtime.getAsBoolean() ? 64 * 1024 : CHUNK);
                if (chunk.length < count) chunk = new byte[count];
                int read = 0;
                while (read < count) { upload.check(); int n = input.read(chunk, read, count - read); if (n < 0) throw new Permanent(R.string.ui_transfer_source_changed); read += n; }
                long started = SystemClock.elapsedRealtime();
                for (int failures = 0; ; failures++) {
                    try {
                        state = upload.request(path, "PUT", chunk, count, confirmed);
                        long next = offset(state, size);
                        if (next != confirmed + count) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
                        confirmed = next; break;
                    } catch (Permanent e) { throw e; }
                    catch (Exception e) {
                        if (failures >= 5) throw e;
                        progress.update(name, (int)(confirmed * 100 / size), true); upload.pause(failures);
                        try {
                            state = upload.request(path, "GET", null, 0, 0);
                            long next = offset(state, size);
                            if (next == confirmed + count) { confirmed = next; break; }
                            if (next != confirmed) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
                        } catch (Permanent permanent) { throw permanent; }
                        catch (Exception retry) { upload.check(); }
                    }
                }
                progress.update(name, (int)(confirmed * 100 / size), false);
                if (realtime.getAsBoolean()) {
                    long remaining = count * 1000L / (1024 * 1024) - (SystemClock.elapsedRealtime() - started);
                    if (remaining > 0) Thread.sleep(remaining);
                }
            }
            for (int attempt = 0; ; attempt++) {
                try {
                    state = upload.request(path, "POST", null, 0, 0);
                    if (offset(state, size) != size || !state.getBoolean("complete") || state.isNull("name")) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
                    completed = true; break;
                } catch (Permanent e) { throw e; }
                catch (Exception e) { if (attempt >= 5) throw e; upload.pause(attempt); }
            }
            progress.update(name, 100, false);
        } finally {
            try {
                if (!completed) transport.deleteAsync(path);
            } finally {
                transport.close();
            }
        }
    }
}
