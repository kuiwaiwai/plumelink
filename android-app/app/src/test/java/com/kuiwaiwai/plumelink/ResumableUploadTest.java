package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;
import java.io.*;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.cert.Certificate;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;
import javax.net.ssl.HttpsURLConnection;
import org.json.JSONObject;

public class ResumableUploadTest {
    @Test public void confirmedReceiptWinsOverSourceCloseFailure() throws Exception {
        java.util.concurrent.atomic.AtomicInteger completed=new java.util.concurrent.atomic.AtomicInteger();
        ResumableUpload.Receipt receipt=ResumableUpload.finishSource(() -> {throw new IOException("descriptor close failed");},
            () -> new ByteArrayInputStream(new byte[0]) {@Override public void close()throws IOException{throw new IOException("source close failed");}},
            input -> new ResumableUpload.Receipt("saved.bin",false), value -> completed.incrementAndGet());
        assertEquals("saved.bin",receipt.name());assertEquals(1,completed.get());
        assertThrows(IOException.class,() -> ResumableUpload.finishSource(() -> {},() -> new ByteArrayInputStream(new byte[0]),input -> {throw new IOException("send failed");},value -> completed.incrementAndGet()));
        assertEquals(1,completed.get());
    }
    @Test public void receiptIsRecordedBeforeAProviderCloseReturns() throws Exception {
        TransferTask task=new TransferTask("pc","owner","usb",true);
        CountDownLatch closing=new CountDownLatch(1),resume=new CountDownLatch(1);
        ExecutorService worker=Executors.newSingleThreadExecutor();
        try {
            Future<ResumableUpload.Receipt> result=worker.submit(() -> ResumableUpload.finishSource(() -> {},
                () -> new ByteArrayInputStream(new byte[0]) {@Override public void close(){closing.countDown();try{resume.await();}catch(InterruptedException error){Thread.currentThread().interrupt();}}},
                input -> new ResumableUpload.Receipt("saved.bin",false),value -> task.uploadCompleted(value.name())));
            assertTrue(closing.await(2,TimeUnit.SECONDS));assertEquals(1,task.completed);assertFalse(task.uploadPending);assertEquals("saved.bin",task.name);assertFalse(result.isDone());
            resume.countDown();assertEquals("saved.bin",result.get(2,TimeUnit.SECONDS).name());assertEquals(1,task.completed);
        } finally {resume.countDown();worker.shutdownNow();}
    }
    @Test public void cancellationBeforeRequestRegistrationNeverStartsTheCall() throws Exception {
        AtomicInteger requests = new AtomicInteger();
        okhttp3.OkHttpClient client = new okhttp3.OkHttpClient.Builder().addInterceptor(chain -> {
            requests.incrementAndGet();
            return new okhttp3.Response.Builder().request(chain.request()).protocol(okhttp3.Protocol.HTTP_1_1)
                .code(200).message("OK").body(okhttp3.ResponseBody.create("{}", null)).build();
        }).build();
        AtomicBoolean cancelled = new AtomicBoolean();
        AtomicReference<Closeable> active = new AtomicReference<>();
        okhttp3.Call call = client.newCall(new okhttp3.Request.Builder().url("http://localhost/api/uploads/task").build());
        cancelled.set(true);
        assertThrows(TransferFailure.class, () -> ResumableUpload.exchange(call, cancelled, active));
        assertTrue(call.isCanceled());
        assertEquals(0, requests.get());
        assertNull(active.get());
    }

    @Test public void cancellationDuringConfirmationReadingReleasesTheRequestAndAllowsRetry() throws Exception {
        AtomicBoolean cancelled = new AtomicBoolean();
        AtomicReference<Closeable> active = new AtomicReference<>();
        CountDownLatch readingBody = new CountDownLatch(1);
        okhttp3.OkHttpClient client = new okhttp3.OkHttpClient.Builder().readTimeout(30, TimeUnit.SECONDS)
            .eventListener(new okhttp3.EventListener() {
                @Override public void responseBodyStart(okhttp3.Call call) { readingBody.countDown(); }
            }).build();
        ExecutorService workers = Executors.newFixedThreadPool(2);
        CountDownLatch bodyStarted = new CountDownLatch(1), finish = new CountDownLatch(1);
        try (java.net.ServerSocket listener = new java.net.ServerSocket(0, 1, java.net.InetAddress.getByName("127.0.0.1"))) {
            Future<?> server = workers.submit(() -> {
                try (java.net.Socket socket = listener.accept()) {
                    socket.setSoTimeout(3000);
                    BufferedReader headers = new BufferedReader(new InputStreamReader(socket.getInputStream(), StandardCharsets.US_ASCII));
                    String line;
                    while ((line = headers.readLine()) != null && !line.isEmpty()) {}
                    OutputStream response = socket.getOutputStream();
                    response.write("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{".getBytes(StandardCharsets.US_ASCII));
                    response.flush();
                    bodyStarted.countDown();
                    assertTrue(finish.await(5, TimeUnit.SECONDS));
                } catch (Exception error) { throw new RuntimeException(error); }
            });
            okhttp3.Call call = client.newCall(new okhttp3.Request.Builder()
                .url("http://" + listener.getInetAddress().getHostAddress() + ":" + listener.getLocalPort() + "/api/uploads/task").build());
            Future<?> request = workers.submit(() -> {
                assertThrows(IOException.class, () -> ResumableUpload.exchange(call, cancelled, active));
            });
            assertTrue(bodyStarted.await(3, TimeUnit.SECONDS));
            assertTrue(readingBody.await(3, TimeUnit.SECONDS));
            Closeable handle = active.get();
            assertNotNull("cancellation must stay registered until the response body is read", handle);
            cancelled.set(true);
            handle.close();
            request.get(2, TimeUnit.SECONDS);
            assertTrue(call.isCanceled());
            assertNull(active.get());
            finish.countDown();
            server.get(2, TimeUnit.SECONDS);
            cancelled.set(false);
            okhttp3.OkHttpClient retry = client.newBuilder().addInterceptor(chain -> new okhttp3.Response.Builder()
                .request(chain.request()).protocol(okhttp3.Protocol.HTTP_1_1).code(200).message("OK")
                .body(okhttp3.ResponseBody.create("{}", null)).build()).build();
            ResumableUpload.exchange(retry.newCall(new okhttp3.Request.Builder().url("http://localhost/api/uploads/retry").build()), cancelled, active);
            assertNull(active.get());
        } finally {
            finish.countDown();
            Closeable handle = active.get();
            if (handle != null) handle.close();
            workers.shutdownNow();
            assertTrue(workers.awaitTermination(3, TimeUnit.SECONDS));
        }
    }

    @Test public void confirmationKeepsValidUtf8AndTheExactByteLimit() throws Exception {
        String text = "{\"name\":\"羽连\"}";
        byte[] bytes = text.getBytes(StandardCharsets.UTF_8);
        try (okhttp3.ResponseBody body = okhttp3.ResponseBody.create(bytes, null)) {
            assertEquals(text, ResumableUpload.readConfirmation(body));
        }
        String limit = " ".repeat(16 * 1024);
        try (okhttp3.ResponseBody body = okhttp3.ResponseBody.create(limit.getBytes(StandardCharsets.UTF_8), null)) {
            assertEquals(limit, ResumableUpload.readConfirmation(body));
        }
    }

    @Test public void oversizedUnknownLengthConfirmationStopsReadingBeforeBufferingTheWholeBody() throws Exception {
        AtomicInteger consumed = new AtomicInteger();
        AtomicBoolean closed = new AtomicBoolean();
        InputStream input = new InputStream() {
            @Override public int read() { return consumed.getAndIncrement() < 1024 * 1024 ? ' ' : -1; }
            @Override public int read(byte[] target, int offset, int length) {
                int count = Math.min(length, 1024 * 1024 - consumed.get());
                if (count <= 0) return -1;
                Arrays.fill(target, offset, offset + count, (byte)' '); consumed.addAndGet(count); return count;
            }
            @Override public void close() { closed.set(true); }
        };
        try (okhttp3.ResponseBody body = okhttp3.ResponseBody.create(null, -1, okio.Okio.buffer(okio.Okio.source(input)))) {
            try { ResumableUpload.readConfirmation(body); fail("oversized confirmation accepted"); }
            catch (TransferFailure expected) {}
        }
        assertTrue("response was buffered beyond the size limit", consumed.get() <= 16 * 1024 + 8192);
        assertTrue("response stream was not closed", closed.get());
    }

    @Test public void declaredConfirmationLengthMustMatchReceivedBytes() throws Exception {
        for (long length : new long[]{1, 3}) {
            try (okhttp3.ResponseBody body = okhttp3.ResponseBody.create(null, length, new okio.Buffer().writeUtf8("{}"))) {
                try { ResumableUpload.readConfirmation(body); fail("mismatched response length accepted"); }
                catch (IOException expected) {}
            }
        }
    }

    @Test public void batchProgressAdvancesAcrossFilesAndWaitsForCompletion() {
        assertEquals(0, ResumableUpload.batchProgress(0, 3, 0));
        assertEquals(33, ResumableUpload.batchProgress(0, 3, 100));
        assertEquals(33, ResumableUpload.batchProgress(1, 3, 0));
        assertEquals(50, ResumableUpload.batchProgress(1, 3, 50));
        assertEquals(99, ResumableUpload.batchProgress(2, 3, 100));
        assertEquals(99, ResumableUpload.batchProgress(Integer.MAX_VALUE, 3, 100));
        assertEquals(0, ResumableUpload.batchProgress(0, 0, 100));
    }

    private static final class Server implements ResumableUpload.Connections {
        final ByteArrayOutputStream received = new ByteArrayOutputStream();
        final List<Integer> chunks = new ArrayList<>(), writes = new ArrayList<>();
        boolean loseChunkAck, rejectChunk, loseFinishAck, cancelOnPut, wrongOffset, deleted, cancelOnFinish, pendingDelete, denyDelete;
        int publications, puts, gets, deletes;
        long size;
        final AtomicBoolean cancelled = new AtomicBoolean();
        @Override public HttpsURLConnection open(String url, String token) throws Exception {
            assertEquals("test-token", token);
            return new HttpsURLConnection(new URL(url)) {
                final ByteArrayOutputStream body = new ByteArrayOutputStream();
                final Map<String, String> headers = new HashMap<>();
                byte[] response;
                public void disconnect() {}
                public boolean usingProxy() { return false; }
                public void connect() {}
                public String getCipherSuite() { return "test"; }
                public Certificate[] getLocalCertificates() { return new Certificate[0]; }
                public Certificate[] getServerCertificates() { return new Certificate[0]; }
                public void setRequestProperty(String key, String value) { headers.put(key, value); }
                public OutputStream getOutputStream() {
                    return new OutputStream() {
                        @Override public void write(int value) { body.write(value); }
                        @Override public void write(byte[] bytes, int offset, int count) {
                            if ("PUT".equals(getRequestMethod())) writes.add(count);
                            body.write(bytes, offset, count);
                        }
                    };
                }
                public int getResponseCode() throws IOException {
                    try {
                        switch (getRequestMethod()) {
                            case "DELETE": {
                                deleted = true; deletes++;
                                if (denyDelete) return 403;
                                response = new JSONObject().put("offset", received.size()).put("size", size).put("chunkBytes", 4 * 1024 * 1024)
                                    .put("complete", publications == 1).put("name", publications == 1 ? "test.bin" : JSONObject.NULL)
                                    .put("cancelled", publications != 1 && !pendingDelete).put("state", publications == 1 ? "completed" : pendingDelete ? "cancelling" : "cancelled")
                                    .toString().getBytes(StandardCharsets.UTF_8);
                                return pendingDelete ? 202 : 200;
                            }
                            case "GET": gets++; break;
                            case "PUT": {
                                puts++;
                                if (cancelOnPut) { cancelled.set(true); throw new IOException("cancelled during request"); }
                                if (rejectChunk) return 403;
                                byte[] bytes = body.toByteArray();
                                assertEquals(received.size(), Long.parseLong(headers.get("X-Upload-Offset")));
                                assertTrue(bytes.length <= 4 * 1024 * 1024);
                                byte[] digest = MessageDigest.getInstance("SHA-256").digest(bytes);
                                StringBuilder hex = new StringBuilder(); for (byte b : digest) hex.append(String.format(Locale.ROOT, "%02x", b));
                                assertEquals(hex.toString(), headers.get("X-Chunk-SHA256"));
                                received.write(bytes); chunks.add(bytes.length);
                                if (loseChunkAck) { loseChunkAck = false; throw new IOException("write succeeded, response lost"); }
                                break;
                            }
                            case "POST": {
                                if (url.getPath().equals("/api/uploads")) size = new JSONObject(body.toString("UTF-8")).getLong("size");
                                else {
                                    assertEquals(size, received.size());
                                    publications = 1;
                                    if (cancelOnFinish) { cancelled.set(true); throw new IOException("cancel raced with publication"); }
                                    if (loseFinishAck) { loseFinishAck = false; throw new IOException("published, response lost"); }
                                }
                                break;
                            }
                            default: throw new AssertionError(getRequestMethod());
                        }
                        response = new JSONObject().put("offset", received.size() + (wrongOffset && puts > 0 ? 1 : 0))
                            .put("size", size).put("chunkBytes", 4 * 1024 * 1024).put("complete", publications == 1)
                            .put("name", publications == 1 ? "test.bin" : JSONObject.NULL).toString().getBytes(StandardCharsets.UTF_8);
                        return 200;
                    } catch (IOException e) { throw e; }
                    catch (Exception e) { throw new IOException(e); }
                }
                public InputStream getInputStream() { return new ByteArrayInputStream(response); }
            };
        }
        ResumableUpload.Receipt send(byte[] data, boolean realtime) throws Exception {
            AtomicReference<HttpsURLConnection> active = new AtomicReference<>();
            try {
                return ResumableUpload.sendStream(this, "https://localhost", "test-token", new ByteArrayInputStream(data), "test.bin", data.length,
                    cancelled, active, () -> realtime, (name, percent, retrying) -> assertTrue(percent >= 0 && percent <= 100));
            } finally { assertNull(active.get()); }
        }
    }
    @Test public void lostAcknowledgementsRecoverExactBytesAndPublishOnce() throws Exception {
        byte[] source = new byte[4 * 1024 * 1024 + 371]; new Random(19).nextBytes(source);
        Server server = new Server(); server.loseChunkAck = true; server.loseFinishAck = true;
        server.send(source, false);
        assertArrayEquals(source, server.received.toByteArray()); assertEquals(2, server.puts);
        assertEquals(1, server.gets); assertEquals(1, server.publications); assertFalse(server.deleted);
    }
    @Test public void activeAudioUsesSmallChunksAndKeepsTail() throws Exception {
        byte[] source = new byte[128 * 1024 + 13]; new Random(4).nextBytes(source);
        Server server = new Server(); server.send(source, true);
        assertEquals(Arrays.asList(65536, 65536, 13), server.chunks);
        assertArrayEquals(source, server.received.toByteArray());
    }
    @Test public void cancellationRevocationAndBadOffsetsNeverPublish() throws Exception {
        for (int failure = 0; failure < 3; failure++) {
            Server server = new Server(); server.cancelOnPut = failure == 0; server.rejectChunk = failure == 1; server.wrongOffset = failure == 2;
            assertThrows(Exception.class, () -> server.send(new byte[4096], false));
            assertEquals(0, server.publications); assertTrue(server.deleted);
        }
    }
    @Test public void twoGiBPlusOneFailsBeforeNetworkOrAllocation() {
        AtomicBoolean opened = new AtomicBoolean();
        assertThrows(TransferFailure.class, () -> ResumableUpload.sendStream((url, token) -> { opened.set(true); return null; },
            "https://localhost", "test-token", new ByteArrayInputStream(new byte[0]), "large.bin", 2L * 1024 * 1024 * 1024 + 1,
            new AtomicBoolean(), new AtomicReference<>(), () -> false, (n, p, r) -> {}));
        assertFalse(opened.get());
    }
    @Test public void invalidNamesFailBeforeNetwork() {
        for (String name : new String[]{"CON.txt", "file.", "file:stream", ".plumelink-temp", "界".repeat(81)}) {
            AtomicBoolean opened = new AtomicBoolean();
            assertThrows(TransferFailure.class, () -> ResumableUpload.sendStream((url, token) -> {
                opened.set(true); return null;
            }, "https://localhost", "test-token", new ByteArrayInputStream(new byte[]{1}), name, 1,
                new AtomicBoolean(), new AtomicReference<>(), () -> false, (n, p, r) -> {}));
            assertFalse(name, opened.get());
        }
    }
    @Test public void zeroByteFilePublishesWithoutChunkPut() throws Exception {
        Server server = new Server();
        List<Integer> progress = new ArrayList<>();
        AtomicReference<HttpsURLConnection> active = new AtomicReference<>();
        ResumableUpload.sendStream(server, "https://localhost", "test-token", new ByteArrayInputStream(new byte[0]), "test.bin", 0,
            server.cancelled, active, () -> false, (name, percent, retrying) -> progress.add(percent));
        assertEquals(0, server.puts);
        assertEquals(1, server.publications);
        assertFalse(server.deleted);
        assertTrue(progress.contains(100));
    }
    @Test public void completedReceiptWinsWhenCancellationArrivesDuringPublication() throws Exception {
        Server server = new Server(); server.cancelOnFinish = true;
        ResumableUpload.Receipt receipt = server.send(new byte[17], false);
        assertTrue(server.cancelled.get()); assertTrue(receipt.recovered()); assertEquals("test.bin", receipt.name());
        assertEquals(1, server.publications); assertEquals(1, server.deletes);
    }
    @Test public void cancellationWithoutATerminalReceiptIsBoundedAndNeverClaimsSuccess() throws Exception {
        Server server = new Server(); server.cancelOnPut = true; server.pendingDelete = true;
        long began = System.nanoTime();
        TransferFailure failure = assertThrows(ResumableUpload.Unconfirmed.class, () -> server.send(new byte[17], false));
        assertEquals(R.string.ui_upload_result_unconfirmed, failure.reason());
        assertTrue(TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - began) < 5000);
        assertTrue(server.deletes > 1); assertTrue(server.deletes <= 12); assertEquals(0, server.publications);
    }
    @Test public void permissionLossDuringCancellationDoesNotBecomeAFalseCancellationReceipt() throws Exception {
        Server server = new Server(); server.cancelOnPut = true; server.denyDelete = true;
        assertThrows(ResumableUpload.Unconfirmed.class, () -> server.send(new byte[17], false));
        assertEquals(1, server.deletes);
    }
    @Test public void cancellationBeforeAnyRequestDoesNotCreateAnUnneededServerRecord() throws Exception {
        Server server = new Server(); server.cancelled.set(true);
        TransferFailure failure = assertThrows(TransferFailure.class, () -> server.send(new byte[17], false));
        assertEquals(R.string.ui_transfer_canceled, failure.reason()); assertEquals(0, server.deletes); assertEquals(0, server.puts);
    }
    @Test public void legacyAmbiguousCancellationAndPendingStatesAreNotTerminal() throws Exception {
        assertEquals(ResumableUpload.CancelState.PENDING, ResumableUpload.cancelState(new JSONObject().put("cancelled", true), 5));
        assertEquals(ResumableUpload.CancelState.PENDING, ResumableUpload.cancelState(new JSONObject().put("state", "cancelling").put("cancelled", false), 5));
        assertEquals(ResumableUpload.CancelState.CANCELLED, ResumableUpload.cancelState(new JSONObject().put("state", "cancelled").put("complete", false).put("cancelled", true), 5));
    }
    @Test public void legacyNetworkWritesAreBoundedWithoutChangingTheProtocolChunk() throws Exception {
        byte[] source = new byte[256 * 1024 + 13]; new Random(62).nextBytes(source);
        Server server = new Server(); server.send(source, false);
        assertEquals(java.util.Collections.singletonList(source.length), server.chunks);
        assertTrue(server.writes.stream().allMatch(count -> count > 0 && count <= RealtimeUploadPacer.QUANTUM));
        assertEquals(17, server.writes.size()); assertArrayEquals(source, server.received.toByteArray()); assertEquals(1, server.publications);
    }
}
