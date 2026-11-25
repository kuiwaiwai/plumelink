package com.kuiwaiwai.plumelink;

import android.content.ContentResolver;
import android.net.Uri;
import android.webkit.MimeTypeMap;
import java.io.*;
import java.util.Locale;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import okhttp3.Call;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;
import okhttp3.ResponseBody;

final class ResumableDownload {
    static final class Unconfirmed extends TransferFailure {
        Unconfirmed() { super(R.string.ui_download_result_unconfirmed); }
    }
    interface Progress { void update(int percent); }
    static final long MAX_BYTES = 2L * 1024 * 1024 * 1024;
    static int rejectionReason(int status) {
        return status == 401 ? R.string.ui_transfer_pairing_expired
            : status == 403 ? R.string.ui_transfer_permission_denied : R.string.ui_transfer_download_rejected;
    }
    private static final class Permanent extends TransferFailure {
        Permanent(int reason, Object... arguments) { super(reason, arguments); }
    }
    private static String inferMimeType(String name) {
        int dot = name.lastIndexOf('.');
        if (dot > 0 && dot < name.length() - 1) {
            String ext = name.substring(dot + 1).toLowerCase(Locale.ROOT);
            String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);
            if (mime != null && !mime.isEmpty()) return mime;
        }
        return "application/octet-stream";
    }
    static void save(ContentResolver resolver, PinnedConnections network, String url, String token, String treeValue,
            String name, AtomicBoolean cancelled, AtomicReference<Closeable> active, Progress progress, PendingDownloads pending, String origin, String task) throws Exception {
        save(resolver,network,url,token,treeValue,name,cancelled,active,progress,pending,origin,task,"");
    }
    static void save(ContentResolver resolver, PinnedConnections network, String url, String token, String treeValue,
            String name, AtomicBoolean cancelled, AtomicReference<Closeable> active, Progress progress, PendingDownloads pending, String origin, String task, String owner) throws Exception {
        save(resolver,network,url,token,treeValue,name,cancelled,active,progress,pending,origin,task,owner,null);
    }
    private static <T> T provider(TransferTask task, TransferTask.Io<T> operation) throws Exception {
        return task == null ? operation.run() : task.provider(operation);
    }
    private static void close(TransferTask task, OutputStream stream) throws Exception {
        if (stream == null) return;
        if (task == null) stream.close();
        else task.cleanup(() -> { stream.close(); return null; });
    }
    static void save(ContentResolver resolver, PinnedConnections network, String url, String token, String treeValue,
            String name, AtomicBoolean cancelled, AtomicReference<Closeable> active, Progress progress, PendingDownloads pending, String origin, String task, String owner, TransferTask work) throws Exception {
        Uri parent = provider(work, () -> ReceiveDirectory.writableDocument(resolver, treeValue));
        if (parent == null) throw new Permanent(R.string.ui_android_download_folder_unavailable);
        OkHttpClient client = network.transferClient().newBuilder().readTimeout(90, TimeUnit.SECONDS).build();
        PendingDownloads.Target destination = null; OutputStream output = null;
        long total = 0, expected = -1, began = android.os.SystemClock.elapsedRealtime(), lastProgress = 0, progressSinceRetry = 0;
        String etag = null; int failures = 0;
        try {
            while (expected < 0 || total < expected) {
                if (cancelled.get()) throw new Permanent(R.string.ui_transfer_canceled);
                if (android.os.SystemClock.elapsedRealtime() - began > 2 * 60 * 60 * 1000L) throw new Permanent(R.string.ui_transfer_timeout);
                Request.Builder builder = new Request.Builder()
                    .url(url)
                    .header("Authorization", "Bearer " + token)
                    .header("Accept-Encoding", "identity");
                if (expected >= 0) {
                    builder.header("Range", "bytes=" + total + "-").header("If-Range", etag);
                }
                Call call = client.newCall(builder.build());
                Closeable cancelHandle = call::cancel;
                active.set(cancelHandle);
                if (cancelled.get()) { call.cancel(); throw new Permanent(R.string.ui_transfer_canceled); }
                try (Response response = call.execute()) {
                    int status = response.code();
                    if (status != (expected < 0 ? 200 : 206)) {
                        if (status == 410) throw new Permanent(R.string.ui_usb_connection_expired);
                        if (status == 429 || status >= 500) throw new IOException("Retryable HTTP status " + status);
                        throw new Permanent(rejectionReason(status), status);
                    }
                    ResponseBody body = response.body();
                    if (body == null) throw new EOFException("Missing download body");
                    long contentLength = body.contentLength();
                    if (expected < 0) {
                        expected = contentLength;
                        etag = response.header("ETag");
                        if (expected < 0 || expected > MAX_BYTES) throw new Permanent(R.string.ui_transfer_invalid_download_size);
                        if (expected > 0 && etag == null) {
                            // Single-shot stream without ETag cannot resume safely if interrupted mid-transfer.
                            etag = "";
                        }
                        try {
                            destination = pending.prepare(origin, treeValue, name, expected, task, owner);
                            PendingDownloads.Target target = destination;
                            output = provider(work, () -> target.open(resolver, parent, inferMimeType(name), cancelled::get));
                            if (work != null) work.local.set(output);
                        } catch (FileNotFoundException | SecurityException | IllegalArgumentException e) {
                            throw new Permanent(R.string.ui_transfer_destination_unwritable);
                        }
                        if (output == null) throw new Permanent(R.string.ui_transfer_destination_unwritable);
                        if (expected == 0) break;
                    } else if (etag == null || etag.isEmpty() || !etag.equals(response.header("ETag")) || contentLength != expected - total
                        || !("bytes " + total + "-" + (expected - 1) + "/" + expected).equals(response.header("Content-Range"))) {
                        throw new Permanent(R.string.ui_transfer_download_changed);
                    }
                    byte[] buffer = new byte[64 * 1024];
                    try (InputStream input = body.byteStream()) {
                        while (total < expected) {
                            if (cancelled.get()) throw new Permanent(R.string.ui_transfer_canceled);
                            if (android.os.SystemClock.elapsedRealtime() - began > 2 * 60 * 60 * 1000L) throw new Permanent(R.string.ui_transfer_timeout);
                            int count = input.read(buffer, 0, (int)Math.min(buffer.length, expected - total));
                            if (count < 0) throw new EOFException("Incomplete download body");
                            try {
                                OutputStream target = output;
                                provider(work, () -> { if (cancelled.get()) throw new Permanent(R.string.ui_transfer_canceled); target.write(buffer, 0, count); return null; });
                            } catch (IOException e) { throw new Permanent(R.string.ui_transfer_destination_unwritable); }
                            total += count;
                            progressSinceRetry += count;
                            if (progressSinceRetry >= 262_144) {
                                failures = 0;
                                progressSinceRetry = 0;
                            }
                            long now = android.os.SystemClock.elapsedRealtime();
                            if (now - lastProgress >= 200) { lastProgress = now; progress.update((int)(total * 100 / expected)); }
                        }
                    }
                } catch (Permanent e) { throw e; }
                catch (IOException e) {
                    if (cancelled.get()) throw new Permanent(R.string.ui_transfer_canceled);
                    if (++failures > 5) throw new Permanent(R.string.ui_transfer_resume_failed);
                    progressSinceRetry = 0;
                    Thread.sleep(Math.min(4000, 500L << (failures - 1)));
                } finally { active.compareAndSet(cancelHandle, null); }
            }
            if (cancelled.get()) throw new Permanent(R.string.ui_transfer_canceled);
            try {
                OutputStream target = output;
                provider(work, () -> { target.flush(); return null; });
                close(work,output);
                output = null;
            } catch (IOException e) {
                throw new Permanent(R.string.ui_transfer_destination_unwritable);
            }
            if (cancelled.get()) throw new Permanent(R.string.ui_transfer_canceled);
            PendingDownloads.Target target = destination;
            provider(work, () -> { target.publish(cancelled::get); return null; });
            progress.update(100);
        } catch (Exception e) {
            if (output != null) try { close(work,output); } catch (Exception ignored) {} output = null;
            if (destination != null) {
                PendingDownloads.Target target = destination;
                if (work == null) target.abort();
                else work.cleanup(() -> { target.abort(); return null; });
            }
            if (destination != null && destination.outcomeUnconfirmed()) throw new Unconfirmed();
            throw e;
        } finally {
            if (output != null) try { close(work,output); } catch (Exception ignored) {}
            if (work != null) work.local.set(null);
            if (destination != null) destination.release();
        }
    }
}
