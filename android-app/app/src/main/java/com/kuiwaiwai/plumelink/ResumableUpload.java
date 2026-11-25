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
    static int batchProgress(int completed, int total, int currentPercent) {
        if (total <= 0) return 0;
        long percent = ((long) completed * 100 + Math.max(0, Math.min(100, currentPercent))) / total;
        return (int) Math.max(0, Math.min(99, percent));
    }
    interface Progress { void update(String name, int percent, boolean retrying); }
    interface QuantumWriter { void write(byte[] bytes, int offset, int count) throws IOException; }
    static void writePaced(byte[] bytes, int count, RealtimeUploadPacer pacer, QuantumWriter writer) throws IOException {
        if (count < 0 || count > bytes.length) throw new IllegalArgumentException("Invalid upload body length");
        try {
            if (count == 0) pacer.beforeWrite(0);
            for (int offset = 0; offset < count; offset += RealtimeUploadPacer.QUANTUM) {
                int quantum = Math.min(RealtimeUploadPacer.QUANTUM, count - offset);
                pacer.beforeWrite(quantum);
                writer.write(bytes, offset, quantum);
            }
        } catch (InterruptedException error) {
            Thread.currentThread().interrupt();
            throw new Permanent(R.string.ui_transfer_canceled);
        }
    }
    interface Connections { HttpsURLConnection open(String url, String token) throws Exception; }
    private static final class ResponseData {
        final int status;
        final String body;
        ResponseData(int status, String body) { this.status = status; this.body = body; }
    }
    private interface Transport {
        ResponseData send(String path, String method, Map<String, String> headers, byte[] bytes, int count) throws Exception;
        ResponseData cancel(String path, long deadlineNanos) throws Exception;
        void close();
    }
    private static final int CHUNK = 4 * 1024 * 1024;
    private static final int MAX_CONFIRMATION_BYTES = 16 * 1024;
    static int rejectionReason(int status) {
        return status == 401 ? R.string.ui_transfer_pairing_expired
            : status == 403 ? R.string.ui_transfer_permission_denied : R.string.ui_transfer_upload_rejected;
    }
    private static final class Permanent extends TransferFailure {
        Permanent(int reason, Object... arguments) { super(reason, arguments); }
    }
    static String readConfirmation(ResponseBody body) throws IOException {
        long expected = body.contentLength();
        if (expected > MAX_CONFIRMATION_BYTES) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
        try (InputStream input = body.byteStream(); ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[2048];
            int count;
            while ((count = input.read(buffer, 0, Math.min(buffer.length, MAX_CONFIRMATION_BYTES - output.size() + 1))) != -1) {
                if (output.size() + count > MAX_CONFIRMATION_BYTES) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
                output.write(buffer, 0, count);
            }
            if (expected >= 0 && output.size() != expected) throw new IOException("Incomplete confirmation body");
            return output.toString(StandardCharsets.UTF_8.name());
        }
    }
    static ResponseData exchange(Call call, AtomicBoolean cancelled, AtomicReference<Closeable> active) throws IOException {
        Closeable cancelHandle = call::cancel;
        active.set(cancelHandle);
        try {
            if (cancelled.get() || Thread.currentThread().isInterrupted()) {
                call.cancel();
                throw new Permanent(R.string.ui_transfer_canceled);
            }
            try (Response response = call.execute()) {
                ResponseBody body = response.body();
                return new ResponseData(response.code(), body == null ? "" : readConfirmation(body));
            }
        } finally {
            active.compareAndSet(cancelHandle, null);
        }
    }
    record Receipt(String name, boolean recovered) {}
    static final class Unconfirmed extends TransferFailure {
        Unconfirmed() { super(R.string.ui_upload_result_unconfirmed); }
    }
    enum CancelState { COMPLETED, CANCELLED, PENDING }
    static CancelState cancelState(JSONObject state, long expected) throws Exception {
        if (state.optBoolean("complete") && offset(state, expected) == expected && validName(state.optString("name"))) return CancelState.COMPLETED;
        if (!state.optBoolean("complete") && state.optBoolean("cancelled") && "cancelled".equals(state.optString("state"))) return CancelState.CANCELLED;
        return CancelState.PENDING;
    }
    private record Resolution(CancelState state, String name) {}
    private static Resolution reconcile(Transport transport, String path, long size) {
        boolean interrupted = Thread.interrupted();
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(4);
        try {
            for (int attempt = 0; attempt < 12 && System.nanoTime() < deadline; attempt++) {
                try {
                    ResponseData response = transport.cancel(path, deadline);
                    if (response.status == 200 || response.status == 202) {
                        JSONObject state = new JSONObject(response.body);
                        CancelState result = cancelState(state, size);
                        if (result != CancelState.PENDING) return new Resolution(result, state.optString("name"));
                    } else if (response.status == 401 || response.status == 403 || response.status == 410) break;
                } catch (Exception ignored) {}
                long remaining = deadline - System.nanoTime();
                if (remaining <= 0) break;
                try { TimeUnit.NANOSECONDS.sleep(Math.min(remaining, TimeUnit.MILLISECONDS.toNanos(200))); }
                catch (InterruptedException ignored) { interrupted = true; }
            }
            return new Resolution(CancelState.PENDING, "");
        } finally { if (interrupted) Thread.currentThread().interrupt(); }
    }
    private final Transport transport;
    private final String base, token;
    private final AtomicBoolean cancelled;
    private boolean started;
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
        started = true;
        ResponseData response = transport.send(path, method, headers, bytes, count);
        int status = response.status;
        if (status != 200 && status != 201) {
            if (status == 410) throw new Permanent(R.string.ui_usb_connection_expired);
            if (status == 400 || status == 401 || status == 403 || status == 404 || status == 413) throw new Permanent(rejectionReason(status), status);
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
    record Metadata(String name, long size) {}
    static void checkMetadataCancellation(android.os.CancellationSignal signal, BooleanSupplier cancelled) throws TransferFailure {
        if (cancelled.getAsBoolean()) throw new TransferFailure(R.string.ui_transfer_canceled);
        if (signal != null) signal.throwIfCanceled();
    }
    static Metadata readMetadata(ContentResolver resolver, Uri uri, android.os.CancellationSignal signal) throws Exception {
        return readMetadata(resolver, uri, signal, () -> false);
    }
    static Metadata readMetadata(ContentResolver resolver, Uri uri, android.os.CancellationSignal signal, BooleanSupplier cancelled) throws Exception {
        checkMetadataCancellation(signal, cancelled);
        String name = null; long size = -1;
        try (Cursor cursor = resolver.query(uri, new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE}, null, null, null, signal)) {
            if (cursor != null && cursor.moveToFirst()) {
                name = cursor.getString(0);
                if (!cursor.isNull(1)) size = cursor.getLong(1);
            }
        }
        checkMetadataCancellation(signal, cancelled);
        if (size < 0) {
            try (android.content.res.AssetFileDescriptor afd = resolver.openAssetFileDescriptor(uri, "r", signal)) {
                if (afd != null && afd.getLength() >= 0) size = afd.getLength();
            } catch (Exception ignored) {}
            checkMetadataCancellation(signal, cancelled);
        }
        return new Metadata(name, size);
    }
    static void validateMetadata(Metadata value) throws TransferFailure {
        if (value.size() < 0) throw new Permanent(R.string.ui_transfer_unknown_file_size);
        if (!validName(value.name())) throw new Permanent(R.string.ui_transfer_invalid_file_name);
        if (value.size() > ResumableDownload.MAX_BYTES) throw new Permanent(R.string.ui_transfer_file_too_large);
    }
    static Metadata metadata(ContentResolver resolver, Uri uri, android.os.CancellationSignal signal) throws Exception {
        return metadata(resolver, uri, signal, () -> false);
    }
    static Metadata metadata(ContentResolver resolver, Uri uri, android.os.CancellationSignal signal, BooleanSupplier cancelled) throws Exception {
        Metadata value = readMetadata(resolver, uri, signal, cancelled);
        validateMetadata(value);
        return value;
    }
    static Receipt send(ContentResolver resolver, PinnedConnections network, String base, String token, Uri uri,
            AtomicBoolean cancelled, AtomicReference<Closeable> active, BooleanSupplier realtime, Progress progress) throws Exception {
        return send(resolver, network, base, token, uri, null, cancelled, active, realtime, progress);
    }
    static Receipt send(ContentResolver resolver, PinnedConnections network, String base, String token, Uri uri, Metadata expected,
            AtomicBoolean cancelled, AtomicReference<Closeable> active, BooleanSupplier realtime, Progress progress) throws Exception {
        return send(resolver,network,base,token,uri,expected,cancelled,active,realtime,progress,null);
    }
    static Receipt send(ContentResolver resolver, PinnedConnections network, String base, String token, Uri uri, Metadata expected,
            TransferTask task, BooleanSupplier realtime, Progress progress) throws Exception {
        return send(resolver,network,base,token,uri,expected,task.cancelled,task.active,realtime,progress,task);
    }
    private static <T> T provider(TransferTask task, TransferTask.Io<T> operation) throws Exception {
        return task == null ? operation.run() : task.provider(operation);
    }
    private static InputStream source(TransferTask task, InputStream input) {
        if (task == null) return input;
        return new FilterInputStream(input) {
            @Override public int read(byte[] bytes,int offset,int count) throws IOException {
                try { return task.provider(() -> in.read(bytes,offset,count)); }
                catch (IOException error) { throw error; }
                catch (Exception error) { throw new IOException(error); }
            }
            @Override public void close() throws IOException {
                try { task.cleanup(() -> { in.close(); return null; }); }
                catch (IOException error) { throw error; }
                catch (Exception error) { throw new IOException(error); }
            }
        };
    }
    interface SourceSend { Receipt send(InputStream input) throws Exception; }
    static Receipt finishSource(Closeable descriptor, java.util.concurrent.Callable<InputStream> open, SourceSend send,
            java.util.function.Consumer<Receipt> confirmed) throws Exception {
        Receipt receipt=null;
        try (descriptor; InputStream input=open.call()) {
            receipt=send.send(input);
            confirmed.accept(receipt);
        } catch (Exception error) { if (receipt==null) throw error; }
        return receipt;
    }
    private static Receipt send(ContentResolver resolver, PinnedConnections network, String base, String token, Uri uri, Metadata expected,
            AtomicBoolean cancelled, AtomicReference<Closeable> active, BooleanSupplier realtime, Progress progress, TransferTask task) throws Exception {
        android.os.CancellationSignal signal = new android.os.CancellationSignal();
        if (task != null) { task.uploadPending=false; task.providerCancellation(signal::cancel); }
        try {
        Metadata metadata = provider(task, () -> metadata(resolver, uri, signal, cancelled::get));
        if (expected != null && !expected.equals(metadata)) throw new Permanent(R.string.ui_share_file_changed);
        if (cancelled.get()) throw new Permanent(R.string.ui_transfer_canceled);
        String name = metadata.name(); long size = metadata.size();
        android.content.res.AssetFileDescriptor descriptor = provider(task, () -> resolver.openAssetFileDescriptor(uri,"r",signal));
        if (descriptor == null) throw new Permanent(R.string.ui_share_unreadable);
        Closeable descriptorHandle = () -> {
            if (task == null) descriptor.close();
            else try { task.cleanup(() -> { descriptor.close(); return null; }); }
            catch (IOException error) { throw error; }
            catch (Exception error) { throw new IOException(error); }
        };
        return finishSource(descriptorHandle, () -> source(task,provider(task,descriptor::createInputStream)), input -> {
            if (task != null) { task.local.set(descriptor); task.check(); }
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
                                writePaced(bytes, count, new RealtimeUploadPacer(realtime, cancelled::get),
                                    (buffer, offset, length) -> sink.write(buffer, offset, length));
                            }
                        };
                    } else if ("POST".equals(method)) {
                        body = RequestBody.create(new byte[0], null);
                    }
                    req.method(method, body);
                    if (task != null) task.uploadPending=true;
                    return exchange(client.newCall(req.build()), cancelled, active);
                }
                @Override public ResponseData cancel(String path, long deadlineNanos) throws Exception {
                    long remaining = deadlineNanos - System.nanoTime();
                    if (remaining <= 0) throw new IOException("Cancellation confirmation timed out");
                    OkHttpClient shortClient = client.newBuilder().callTimeout(remaining, TimeUnit.NANOSECONDS)
                        .connectTimeout(Math.min(remaining, TimeUnit.SECONDS.toNanos(2)), TimeUnit.NANOSECONDS)
                        .readTimeout(Math.min(remaining, TimeUnit.SECONDS.toNanos(2)), TimeUnit.NANOSECONDS).build();
                    Request req = new Request.Builder().url(base + path).header("Authorization", "Bearer " + token).delete().build();
                    try (Response response = shortClient.newCall(req).execute()) {
                        ResponseBody body = response.body();
                        return new ResponseData(response.code(), body == null ? "" : readConfirmation(body));
                    }
                }
                @Override public void close() {}
            };
            return sendWithTransport(okTransport, base, token, input, name, size, cancelled, realtime, progress);
        }, receipt -> { if (task != null) task.uploadCompleted(receipt.name()); });
        } finally { if (task != null) { task.local.set(null); task.providerCancel.set(null); } }
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
    static Receipt sendStream(Connections network, String base, String token, InputStream input, String name, long size,
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
                            writePaced(bytes, count, new RealtimeUploadPacer(realtime, cancelled::get), output::write);
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
                                if (out.size() + n > MAX_CONFIRMATION_BYTES) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
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
            @Override public ResponseData cancel(String path, long deadlineNanos) throws Exception {
                long remaining = deadlineNanos - System.nanoTime();
                if (remaining <= 0) throw new IOException("Cancellation confirmation timed out");
                HttpsURLConnection cleanup = network.open(base + path, token);
                ScheduledFuture<?> timeout = timeouts.schedule(cleanup::disconnect, remaining, TimeUnit.NANOSECONDS);
                try {
                    int timeoutMillis = (int)Math.max(1, Math.min(2000, TimeUnit.NANOSECONDS.toMillis(remaining)));
                    cleanup.setConnectTimeout(timeoutMillis); cleanup.setReadTimeout(timeoutMillis); cleanup.setRequestMethod("DELETE");
                    int status = cleanup.getResponseCode();
                    String body = "";
                    if (status == 200 || status == 202) {
                        try (InputStream input = cleanup.getInputStream(); ByteArrayOutputStream output = new ByteArrayOutputStream()) {
                            byte[] buffer = new byte[2048]; int count;
                            while ((count = input.read(buffer)) != -1) {
                                if (output.size() + count > MAX_CONFIRMATION_BYTES) throw new IOException("Invalid cancellation confirmation");
                                output.write(buffer, 0, count);
                            }
                            body = output.toString(StandardCharsets.UTF_8.name());
                        }
                    }
                    return new ResponseData(status, body);
                } finally { timeout.cancel(false); cleanup.disconnect(); }
            }
            @Override public void close() { timeouts.shutdownNow(); }
        };
        return sendWithTransport(legacyTransport, base, token, input, name, size, cancelled, realtime, progress);
    }
    private static Receipt sendWithTransport(Transport transport, String base, String token, InputStream input, String name, long size,
            AtomicBoolean cancelled, BooleanSupplier realtime, Progress progress) throws Exception {
        if (size < 0) throw new Permanent(R.string.ui_transfer_unknown_file_size);
        if (size > ResumableDownload.MAX_BYTES) throw new Permanent(R.string.ui_transfer_file_too_large);
        ResumableUpload upload = new ResumableUpload(transport, base, token, cancelled);
        String id = UUID.randomUUID().toString().replace("-", ""), path = "/api/uploads/" + id;
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
            }
            for (int attempt = 0; ; attempt++) {
                try {
                    state = upload.request(path, "POST", null, 0, 0);
                    if (offset(state, size) != size || !state.getBoolean("complete") || state.isNull("name")) throw new Permanent(R.string.ui_transfer_invalid_confirmation);
                    break;
                } catch (Permanent e) { throw e; }
                catch (Exception e) { if (attempt >= 5) throw e; upload.pause(attempt); }
            }
            progress.update(name, 100, false);
            return new Receipt(state.getString("name"), false);
        } catch (Exception failure) {
            if (!upload.started) throw failure;
            Resolution resolution = reconcile(transport, path, size);
            if (resolution.state() == CancelState.COMPLETED) {
                progress.update(name, 100, false);
                return new Receipt(resolution.name(), true);
            }
            if (resolution.state() == CancelState.PENDING && (cancelled.get() || !(failure instanceof Permanent known
                    && (known.reason() == R.string.ui_transfer_pairing_expired || known.reason() == R.string.ui_transfer_permission_denied || known.reason() == R.string.ui_usb_connection_expired))))
                throw new Unconfirmed();
            throw failure;
        } finally { transport.close(); }
    }
}
