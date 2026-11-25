package com.kuiwaiwai.plumelink;

import org.junit.Test;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import static org.junit.Assert.*;

public class PendingDownloadsTest {
    @Test public void unfinishedRecordsKeepTheirDirectoryUntilResolved() throws Exception {
        Path root = Files.createTempDirectory("plumelink-grants-");
        try {
            PendingDownloads pending = new PendingDownloads(root.toFile());
            String tree = "content://example.documents/tree/first";
            PendingDownloads.Target target = pending.prepare("https://127.0.0.1:17840", tree, "notes.bin", 16, "");
            target.release();
            assertEquals(Collections.singleton(tree), pending.requiredTrees());
            try (java.util.stream.Stream<Path> paths = Files.list(root.resolve("pending-downloads"))) {
                Files.delete(paths.findFirst().orElseThrow());
            }
            assertTrue(pending.requiredTrees().isEmpty());
        } finally {
            try (java.util.stream.Stream<Path> paths = Files.walk(root)) {
                for (Path path : paths.sorted(Comparator.reverseOrder()).toArray(Path[]::new)) Files.delete(path);
            }
        }
    }
    @Test public void unreadableOrCorruptRecoveryMetadataPreventsPermissionPruning() throws Exception {
        Path root = Files.createTempDirectory("plumelink-grants-");
        try {
            PendingDownloads pending = new PendingDownloads(root.toFile());
            assertTrue(pending.requiredTrees().isEmpty());
            Path directory = Files.createDirectory(root.resolve("pending-downloads"));
            Files.write(directory.resolve("a".repeat(32) + ".json"), "invalid".getBytes(StandardCharsets.UTF_8));
            assertNull(pending.requiredTrees());
        } finally {
            try (java.util.stream.Stream<Path> paths = Files.walk(root)) {
                for (Path path : paths.sorted(Comparator.reverseOrder()).toArray(Path[]::new)) Files.delete(path);
            }
        }
    }
    @Test public void duplicateNamesPreserveExtensionsAndDotfiles() {
        assertEquals("report.final (1).PDF", PendingDownloads.numberedName("report.final.PDF", 1));
        assertEquals(".env (2)", PendingDownloads.numberedName(".env", 2));
        assertEquals("archive", PendingDownloads.numberedName("archive", 0));
    }
    @Test public void duplicateNamesStayWithinTheByteLimitWithoutSplittingUnicode() {
        for (String stem : new String[]{"😀".repeat(59), "报告".repeat(39)}) {
            String result = PendingDownloads.numberedName(stem + ".png", 1_000_000);
            assertTrue(result.getBytes(StandardCharsets.UTF_8).length <= 240);
            assertEquals(result, new String(result.getBytes(StandardCharsets.UTF_8), StandardCharsets.UTF_8));
            assertTrue(result.endsWith(" (1000000).png"));
        }
    }
    @Test public void entriesAndTreeLookupDoNotContactAProvider() throws Exception {
        Path root = Files.createTempDirectory("plumelink-entry-list-");
        PendingDownloads pending = new PendingDownloads(root.toFile());
        PendingDownloads.Target first = pending.prepare("https://127.0.0.1:17840", "content://one.documents/tree/first", "one.bin", 1, "");
        PendingDownloads.Target second = pending.prepare("https://127.0.0.1:17840", "content://two.documents/tree/second", "two.bin", 1, "");
        try {
            List<PendingDownloads.Entry> entries = pending.entries(); assertEquals(2, entries.size());
            for (PendingDownloads.Entry entry : entries) assertEquals(entry.tree, pending.treeFor(entry.id));
            assertEquals("", pending.treeFor("../invalid"));
            assertTrue(pending.recoverEntries(null, Collections.emptyList()).isEmpty());
            first.release(); second.release();
            Thread.currentThread().interrupt();
            try { assertTrue(pending.recoverEntries(null, entries).isEmpty()); }
            finally { Thread.interrupted(); }
            assertEquals(2, pending.entries().size());
        } finally {
            first.release(); second.release();
            try (java.util.stream.Stream<Path> paths = Files.walk(root)) { for (Path path : paths.sorted(Comparator.reverseOrder()).toArray(Path[]::new)) Files.deleteIfExists(path); }
        }
    }
    @Test public void cancellationAfterNameLookupPreventsTheRenameMutation() throws Exception {
        java.util.concurrent.atomic.AtomicBoolean cancelled = new java.util.concurrent.atomic.AtomicBoolean();
        java.util.concurrent.atomic.AtomicInteger mutations = new java.util.concurrent.atomic.AtomicInteger();
        PendingDownloads.Publication publication = new PendingDownloads.Publication();
        cancelled.set(true);
        assertThrows(TransferFailure.class, () -> publication.run(cancelled::get, () -> mutations.incrementAndGet()));
        assertEquals(0, mutations.get()); assertFalse(publication.started);
    }
    @Test public void cancellationDuringRenameDoesNotDiscardItsReturnedReceipt() throws Exception {
        java.util.concurrent.atomic.AtomicBoolean cancelled = new java.util.concurrent.atomic.AtomicBoolean();
        PendingDownloads.Publication publication = new PendingDownloads.Publication();
        String uri = publication.run(cancelled::get, () -> { cancelled.set(true); return "content://provider/new-document"; });
        assertEquals("content://provider/new-document", uri); assertTrue(publication.started); assertTrue(cancelled.get());
    }
    @Test public void ambiguousRenameFailureIsMarkedAsStartedRatherThanSafeToDelete() {
        PendingDownloads.Publication publication = new PendingDownloads.Publication();
        assertThrows(java.io.IOException.class, () -> publication.run(() -> false, () -> { throw new java.io.IOException("reply lost"); }));
        assertTrue(publication.started);
    }
}
