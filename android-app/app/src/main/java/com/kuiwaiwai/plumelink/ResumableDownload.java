package com.kuiwaiwai.plumelink;

import android.content.ContentResolver;
import android.net.Uri;
import android.provider.DocumentsContract;
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
    interface Progress { void update(int percent); }
    static final long MAX_BYTES = 2L * 1024 * 1024 * 1024;
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
            String name, AtomicBoolean cancelled, AtomicReference<Closeable> active, Progress progress) throws Exception {
        Uri parent = ReceiveDirectory.writableDocument(resolver, treeValue);
        if (parent == null) throw new Permanent(R.string.ui_android_download_folder_unavailable);
        OkHttpClient client = network.transferClient().newBuilder().readTimeout(90, TimeUnit.SECONDS).build();
        Uri created = null; OutputStream output = null;
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
                        if (status == 429 || status >= 500) throw new IOException("Retryable HTTP status " + status);
                        throw new Permanent(R.string.ui_transfer_download_rejected, status);
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
                            created = DocumentsContract.createDocument(resolver, parent, inferMimeType(name), name);
                            if (created != null) output = resolver.openOutputStream(created, "w");
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
                            try { output.write(buffer, 0, count); } catch (IOException e) { throw new Permanent(R.string.ui_transfer_destination_unwritable); }
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
                output.flush();
                output.close();
                output = null;
            } catch (IOException e) {
                throw new Permanent(R.string.ui_transfer_destination_unwritable);
            }
            progress.update(100);
        } catch (Exception e) {
            if (output != null) try { output.close(); } catch (IOException ignored) {} output = null;
            if (created != null) try { DocumentsContract.deleteDocument(resolver, created); } catch (Exception ignored) {}
            throw e;
        } finally { if (output != null) try { output.close(); } catch (IOException ignored) {} }
    }
}
