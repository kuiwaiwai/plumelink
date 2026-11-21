package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;
import java.io.*;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.cert.Certificate;
import java.util.*;
import java.util.concurrent.atomic.*;
import javax.net.ssl.HttpsURLConnection;
import org.json.JSONObject;

public class ResumableUploadTest {
    private static final class Server implements ResumableUpload.Connections {
        final ByteArrayOutputStream received = new ByteArrayOutputStream();
        final List<Integer> chunks = new ArrayList<>();
        boolean loseChunkAck, rejectChunk, loseFinishAck, cancelOnPut, wrongOffset, deleted;
        int publications, puts, gets;
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
                public OutputStream getOutputStream() { return body; }
                public int getResponseCode() throws IOException {
                    try {
                        switch (getRequestMethod()) {
                            case "DELETE": deleted = true; return 200;
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
        void send(byte[] data, boolean realtime) throws Exception {
            AtomicReference<HttpsURLConnection> active = new AtomicReference<>();
            try {
                ResumableUpload.sendStream(this, "https://localhost", "test-token", new ByteArrayInputStream(data), "test.bin", data.length,
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
}
