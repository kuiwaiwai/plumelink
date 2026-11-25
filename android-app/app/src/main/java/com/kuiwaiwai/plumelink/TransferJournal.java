package com.kuiwaiwai.plumelink;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.UUID;
import java.util.Arrays;
import java.util.Comparator;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import org.json.JSONObject;

final class TransferJournal {
    private static final Object LOCK = new Object();
    private static final Set<String> LIVE = ConcurrentHashMap.newKeySet();
    private static final int LIMIT = 16;
    private final File marker, tasks;

    TransferJournal(File directory) {
        marker = new File(directory, "transfer.active");
        tasks = new File(directory, "transfer-tasks");
    }

    static final class Entry {
        final String id, origin, owner;
        Entry(String id, String origin) { this(id,origin,""); }
        Entry(String id, String origin, String owner) { this.id = id; this.origin = origin; this.owner = owner; }
    }

    Entry recover() {
        synchronized (LOCK) {
            try { migrateLegacy(); } catch (IOException ignored) { Entry entry = read(marker); return entry != null && !LIVE.contains(entry.id) ? entry : null; }
            for (File file : taskFiles()) { Entry entry = read(file); if (entry != null && !LIVE.contains(entry.id)) return entry; }
            return null;
        }
    }
    private File taskFile(String id) { return new File(tasks, id + ".json"); }
    private File[] taskFiles() {
        File[] files = tasks.listFiles((dir, name) -> name.matches("[a-f0-9]{32}\\.json"));
        if (files == null) return new File[0];
        Arrays.sort(files, Comparator.comparingLong(File::lastModified).thenComparing(File::getName));
        return files;
    }
    private Entry read(File source) {
        try {
            if (!source.isFile() || source.length() > 2048) return null;
            JSONObject value = new JSONObject(new String(Files.readAllBytes(source.toPath()), StandardCharsets.UTF_8));
            String id = value.getString("id"), origin = value.getString("origin");
            if (!id.matches("[a-f0-9]{32}") || !origin.startsWith("https://") || origin.length() > 512
                    || (!source.equals(marker) && !source.getName().equals(id + ".json"))) return null;
            String owner = value.optString("owner");
            if (!owner.isEmpty() && !owner.matches("[a-f0-9]{64}")) return null;
            return new Entry(id, origin, owner);
        } catch (Exception ignored) { return null; }
    }
    private void migrateLegacy() throws IOException {
        Entry legacy = read(marker);
        if (legacy == null) { Files.deleteIfExists(marker.toPath()); return; }
        File target = taskFile(legacy.id);
        Entry existing = read(target);
        if (existing == null) {
            if (taskFiles().length >= LIMIT) throw new IOException("Transfer recovery records are full");
            write(target, legacy);
        } else if (!existing.origin.equals(legacy.origin) || !existing.owner.equals(legacy.owner)) {
            throw new IOException("Conflicting transfer recovery record");
        }
        Files.deleteIfExists(marker.toPath());
    }
    private static void write(File file, Entry entry) throws IOException {
        try {
            JSONObject value = new JSONObject().put("id", entry.id).put("origin", entry.origin);
            if (!entry.owner.isEmpty()) value.put("owner", entry.owner);
            save(file, value);
        } catch (org.json.JSONException error) { throw new IOException("Could not save transfer state", error); }
    }
    Lease begin(String origin) throws IOException { return begin(origin, ""); }
    Lease begin(String origin, String owner) throws IOException {
        synchronized (LOCK) {
            if (origin == null || !origin.startsWith("https://") || origin.length() > 512
                    || owner == null || !(owner.isEmpty() || owner.matches("[a-f0-9]{64}"))) throw new IOException("Invalid transfer identity");
            migrateLegacy();
            for (File file : taskFiles()) if (read(file) == null) Files.deleteIfExists(file.toPath());
            if (taskFiles().length >= LIMIT) throw new IOException("Transfer recovery records are full");
            Entry entry = new Entry(UUID.randomUUID().toString().replace("-", ""), origin, owner);
            LIVE.add(entry.id);
            try { write(taskFile(entry.id), entry); return new Lease(entry); }
            catch (IOException error) { LIVE.remove(entry.id); throw error; }
        }
    }

    static void save(File destination, JSONObject value) throws IOException {
        synchronized (LOCK) {
            File temporary = new File(destination.getParentFile(), destination.getName() + ".tmp");
            try {
                Files.createDirectories(destination.getParentFile().toPath());
                byte[] bytes = value.toString().getBytes(StandardCharsets.UTF_8);
                try (FileOutputStream stream = new FileOutputStream(temporary)) {
                    stream.write(bytes);
                    stream.getFD().sync();
                }
                try {
                    Files.move(temporary.toPath(), destination.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
                } catch (AtomicMoveNotSupportedException ignored) {
                    Files.move(temporary.toPath(), destination.toPath(), StandardCopyOption.REPLACE_EXISTING);
                }
            }
            finally { temporary.delete(); }
        }
    }

    final class Lease implements AutoCloseable {
        private final Entry entry;
        private final AtomicBoolean closed = new AtomicBoolean();
        Lease(Entry entry) { this.entry = entry; }
        String id() { return entry.id; }
        String owner() { return entry.owner; }
        @Override public void close() throws IOException {
            if (!closed.compareAndSet(false, true)) return;
            try { acknowledge(entry.id, true); }
            finally { LIVE.remove(entry.id); }
        }
    }
    boolean acknowledge(String id) throws IOException { return acknowledge(id, false); }
    private boolean acknowledge(String id, boolean ownLease) throws IOException {
        if (id == null || !id.matches("[a-f0-9]{32}")) return false;
        synchronized (LOCK) {
            if (!ownLease && LIVE.contains(id)) return false;
            boolean removed = false;
            Entry legacy = read(marker);
            if (legacy != null && legacy.id.equals(id)) { Files.deleteIfExists(marker.toPath()); removed = true; }
            File target = taskFile(id);
            if (read(target) != null) { Files.deleteIfExists(target.toPath()); removed = true; }
            return removed;
        }
    }
}
