package com.kuiwaiwai.plumelink;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import org.json.JSONObject;
import static org.junit.Assert.*;

public class TransferJournalTest {
    private Path directory;
    private TransferJournal journal;
    private static final String ORIGIN = "https://127.0.0.1:17840";
    @Before public void create() throws Exception {
        directory = Files.createTempDirectory("plumelink-journal-"); journal = new TransferJournal(directory.toFile());
    }
    @After public void remove() throws Exception {
        try (java.util.stream.Stream<Path> paths = Files.walk(directory)) {
            for (Path path : paths.sorted(Comparator.reverseOrder()).toArray(Path[]::new)) Files.deleteIfExists(path);
        }
    }
    private Path file(String id) { return directory.resolve("transfer-tasks").resolve(id + ".json"); }
    @Test public void coldRecoveryKeepsOnlyTaskIdentityAndOrigin() throws Exception {
        byte[] bytes; String id;
        try (TransferJournal.Lease lease = journal.begin(ORIGIN)) {
            id = lease.id(); bytes = Files.readAllBytes(file(id)); assertNull(journal.recover());
            JSONObject value = new JSONObject(new String(bytes, StandardCharsets.UTF_8));
            assertEquals(2, value.length()); assertTrue(value.has("id") && value.has("origin"));
        }
        Files.write(file(id), bytes);
        TransferJournal.Entry recovered = new TransferJournal(directory.toFile()).recover();
        assertNotNull(recovered); assertEquals(id, recovered.id); assertEquals(ORIGIN, recovered.origin);
    }
    @Test public void normalCompletionAndFailureReleaseOnlyTheirOwnRecords() throws Exception {
        try (TransferJournal.Lease lease = journal.begin(ORIGIN)) { assertTrue(Files.exists(file(lease.id()))); }
        assertNull(journal.recover());
        assertThrows(IOException.class, () -> { try (TransferJournal.Lease ignored = journal.begin(ORIGIN)) { throw new IOException("failure"); } });
        try (java.util.stream.Stream<Path> files = Files.list(directory.resolve("transfer-tasks"))) { assertEquals(0, files.count()); }
    }
    @Test public void lateCleanupCannotEraseANewerTaskAndLiveWorkIsNotRecovered() throws Exception {
        TransferJournal.Lease old = journal.begin(ORIGIN), current = journal.begin("https://127.0.0.2:17840");
        try {
            assertNull(new TransferJournal(directory.toFile()).recover()); assertFalse(journal.acknowledge(old.id()));
            old.close(); assertFalse(Files.exists(file(old.id()))); assertTrue(Files.exists(file(current.id())));
            assertNull(journal.recover());
        } finally { old.close(); current.close(); current.close(); }
        assertNull(journal.recover());
    }
    @Test public void legacyMarkerIsMigratedWithoutLosingOwner() throws Exception {
        String id = "a".repeat(32), owner = "b".repeat(64);
        Files.write(directory.resolve("transfer.active"), new JSONObject().put("id", id).put("origin", ORIGIN).put("owner", owner).toString().getBytes(StandardCharsets.UTF_8));
        TransferJournal.Entry entry = journal.recover(); assertEquals(id, entry.id); assertEquals(owner, entry.owner);
        assertFalse(Files.exists(directory.resolve("transfer.active"))); assertTrue(Files.exists(file(id)));
        assertTrue(journal.acknowledge(id)); assertNull(journal.recover());
    }
    @Test public void acknowledgingOneColdTaskKeepsAllOthersDiscoverable() throws Exception {
        Set<String> expected = new HashSet<>();
        for (int i = 0; i < 3; i++) {
            String id = String.format("%032x", i); expected.add(id);
            Files.createDirectories(file(id).getParent()); Files.write(file(id), new JSONObject().put("id", id).put("origin", ORIGIN).toString().getBytes(StandardCharsets.UTF_8));
        }
        Set<String> found = new HashSet<>(); TransferJournal.Entry entry;
        while ((entry = journal.recover()) != null) { assertTrue(found.add(entry.id)); assertTrue(journal.acknowledge(entry.id)); }
        assertEquals(expected, found);
    }
    @Test public void theJournalHasAHardLimitAndReleasesCapacityOnlyAfterARealLeaseEnds() throws Exception {
        List<TransferJournal.Lease> leases = new ArrayList<>();
        try {
            for (int i = 0; i < 16; i++) leases.add(journal.begin(ORIGIN));
            assertThrows(IOException.class, () -> journal.begin(ORIGIN));
            leases.get(0).close(); leases.add(journal.begin(ORIGIN)); assertNull(journal.recover());
        } finally { for (TransferJournal.Lease lease : leases) lease.close(); }
    }
    @Test public void corruptLegacyStateDoesNotBlockTheNextTransfer() throws Exception {
        Files.write(directory.resolve("transfer.active"), "invalid".getBytes(StandardCharsets.UTF_8)); assertNull(journal.recover());
        try (TransferJournal.Lease lease = journal.begin(ORIGIN)) { assertTrue(Files.exists(file(lease.id()))); }
        assertNull(journal.recover());
    }
    @Test public void unavailableStorageDoesNotChangeExistingData() throws Exception {
        Path blocked = directory.resolve("blocked"); byte[] value = "existing-data".getBytes(StandardCharsets.UTF_8); Files.write(blocked, value);
        TransferJournal unavailable = new TransferJournal(blocked.toFile());
        assertThrows(IOException.class, () -> unavailable.begin(ORIGIN)); assertArrayEquals(value, Files.readAllBytes(blocked)); assertNull(unavailable.recover());
    }
}
