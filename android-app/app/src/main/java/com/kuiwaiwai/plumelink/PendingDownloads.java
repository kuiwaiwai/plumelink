package com.kuiwaiwai.plumelink;

import android.content.ContentResolver;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Callable;
import java.util.function.BooleanSupplier;
import org.json.*;

final class PendingDownloads {
    private static final Set<String> LIVE = ConcurrentHashMap.newKeySet();
    private final File directory;
    PendingDownloads(File files) { directory = new File(files, "pending-downloads"); }

    static final class Entry {
        final String id, origin, tree, name;
        final long expected;
        String uri = "", storedName = "", plannedName = "", task = "", owner = "";
        boolean temporary, complete, publishPending;
        Entry(String id, String origin, String tree, String name, long expected) {
            this.id = id; this.origin = origin; this.tree = tree; this.name = name; this.expected = expected;
        }
    }
    static final class Info {
        final Uri uri;
        final String name;
        final long size, modified;
        final int flags;
        Info(Uri uri, String name, long size, long modified, int flags) {
            this.uri = uri; this.name = name; this.size = size; this.modified = modified; this.flags = flags;
        }
        String version() { return EndpointUtils.sha256Hex(uri + "\n" + name + "\n" + size + "\n" + modified); }
    }
    static final class Recovered {
        final Entry entry;
        final Info info;
        Recovered(Entry entry, Info info) { this.entry = entry; this.info = info; }
        JSONObject json() throws JSONException {
            String folder = DocumentsContract.getTreeDocumentId(Uri.parse(entry.tree));
            if (folder.startsWith("primary:")) folder = folder.substring(8);
            if (folder.length() > 512) {
                int start = folder.length() - 511;
                if (Character.isLowSurrogate(folder.charAt(start))) start++;
                folder = "…" + folder.substring(start);
            }
            return new JSONObject().put("id", entry.id).put("name", info == null ? entry.name : info.name)
                .put("size", info == null ? -1 : info.size).put("expected", entry.expected)
                .put("version", info == null ? "" : info.version()).put("available", info != null).put("folder", folder).put("pendingCommit", entry.publishPending);
        }
    }

    private File file(String id) { return new File(directory, id + ".json"); }
    boolean belongsTo(String id, String owner, String endpoint) {
        if (id == null || !id.matches("[a-f0-9]{32}")) return false;
        Entry entry = read(file(id));
        return entry != null && (!entry.owner.isEmpty() ? entry.owner.equals(owner)
            : !EndpointUtils.isUsbEndpoint(endpoint) && entry.origin.equals(endpoint));
    }
    private void save(Entry e) throws IOException {
        try {
            TransferJournal.save(file(e.id), new JSONObject().put("id", e.id).put("origin", e.origin).put("tree", e.tree)
                .put("name", e.name).put("expected", e.expected).put("uri", e.uri).put("storedName", e.storedName)
                .put("plannedName", e.plannedName).put("temporary", e.temporary).put("complete", e.complete).put("publishPending", e.publishPending).put("task", e.task).put("owner",e.owner));
        } catch (JSONException error) { throw new IOException("Could not save download state", error); }
    }
    private Entry read(File file) {
        try {
            if (!file.getName().matches("[a-f0-9]{32}\\.json") || file.length() > 65536) return null;
            JSONObject json = new JSONObject(new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8));
            String id = json.getString("id"), origin = json.getString("origin"), tree = json.getString("tree"), name = json.getString("name");
            long expected = json.getLong("expected");
            if (!file.getName().equals(id + ".json") || !origin.startsWith("https://") || origin.length() > 512
                || !tree.startsWith("content://") || tree.length() > 16384 || name.isEmpty() || name.length() > 512
                || expected < 0 || expected > ResumableDownload.MAX_BYTES) return null;
            Entry e = new Entry(id, origin, tree, name, expected);
            e.uri = json.optString("uri"); e.storedName = json.optString("storedName"); e.plannedName = json.optString("plannedName");
            e.temporary = json.optBoolean("temporary"); e.complete = json.optBoolean("complete"); e.publishPending = json.optBoolean("publishPending");
            e.task = json.optString("task");
            e.owner = json.optString("owner");
            if (!e.owner.isEmpty() && !e.owner.matches("[a-f0-9]{64}")) return null;
            if (!e.task.isEmpty() && !e.task.matches("[a-f0-9]{32}")) return null;
            if (!e.uri.isEmpty() && (!e.uri.startsWith("content://") || e.uri.length() > 32768
                || !Objects.equals(Uri.parse(tree).getAuthority(), Uri.parse(e.uri).getAuthority()))) return null;
            return e;
        } catch (Exception ignored) { return null; }
    }
    private void forget(Entry e) throws IOException { Files.deleteIfExists(file(e.id).toPath()); }

    Set<String> requiredTrees() {
        Set<String> trees = new HashSet<>();
        File[] files = directory.listFiles((dir, name) -> name.endsWith(".json"));
        if (files == null) return directory.exists() ? null : trees;
        for (File file : files) {
            Entry entry = read(file);
            if (entry == null) return null;
            if (!entry.complete) trees.add(entry.tree);
        }
        return trees;
    }

    Target prepare(String origin, String tree, String name, long expected, String task) throws IOException {
        return prepare(origin,tree,name,expected,task,"");
    }
    Target prepare(String origin, String tree, String name, long expected, String task, String owner) throws IOException {
        Entry e = new Entry(UUID.randomUUID().toString().replace("-", ""), origin, tree, name, expected);
        e.task = task;
        e.owner = owner;
        LIVE.add(e.id);
        try { save(e); return new Target(e); }
        catch (IOException error) { LIVE.remove(e.id); throw error; }
    }

    static Info inspect(ContentResolver resolver, Uri uri) throws IOException {
        try (Cursor cursor = resolver.query(uri, new String[]{Document.COLUMN_DISPLAY_NAME, Document.COLUMN_SIZE,
                Document.COLUMN_LAST_MODIFIED, Document.COLUMN_FLAGS}, null, null, null)) {
            if (cursor == null) throw new IOException("Document provider unavailable");
            if (!cursor.moveToFirst()) return null;
            String name = cursor.getString(0);
            if (name == null || name.isEmpty() || name.length() > 512) throw new IOException("Invalid document name");
            return new Info(uri, name, cursor.isNull(1) ? -1 : cursor.getLong(1),
                cursor.isNull(2) ? 0 : cursor.getLong(2), cursor.getInt(3));
        } catch (RuntimeException error) {
            if (error.getCause() instanceof FileNotFoundException) return null;
            throw new IOException("Document unavailable", error);
        }
    }
    private static boolean owned(Entry e, String name) {
        return name != null && name.matches("\\.plumelink-" + e.id + "\\.part(?:\\.[A-Za-z0-9]{1,16})?");
    }
    private Info locate(ContentResolver resolver, Entry e) throws IOException {
        Info info = null; IOException unavailable = null;
        try { if (!e.uri.isEmpty()) info = inspect(resolver, Uri.parse(e.uri)); }
        catch (IOException error) { unavailable = error; }
        if (info != null) return info;
        if (e.plannedName.isEmpty()) {
            if (unavailable != null) throw unavailable;
            return null;
        }
        Uri tree = Uri.parse(e.tree);
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree));
        try (Cursor cursor = resolver.query(children, new String[]{Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME}, null, null, null)) {
            if (cursor == null) throw new IOException("Document provider unavailable");
            while (cursor.moveToNext()) if (owned(e, cursor.getString(1)))
                return inspect(resolver, DocumentsContract.buildDocumentUriUsingTree(tree, cursor.getString(0)));
        } catch (RuntimeException error) { throw new IOException("Document unavailable", error); }
        return null;
    }

    List<Entry> entries() {
        List<Entry> entries = new ArrayList<>();
        File[] files = directory.listFiles((dir, name) -> name.matches("[a-f0-9]{32}\\.json"));
        if (files == null) return entries;
        Arrays.sort(files, Comparator.comparing(File::getName));
        for (File file : files) { Entry entry = read(file); if (entry != null) entries.add(entry); }
        return entries;
    }
    String treeFor(String id) {
        if (id == null || !id.matches("[a-f0-9]{32}")) return "";
        Entry entry = read(file(id)); return entry == null ? "" : entry.tree;
    }
    List<Recovered> recover(ContentResolver resolver) { return recoverEntries(resolver, entries()); }
    List<Recovered> recoverEntries(ContentResolver resolver, List<Entry> entries) {
        List<Recovered> result = new ArrayList<>();
        for (Entry candidate : entries) {
            if (Thread.currentThread().isInterrupted()) break;
            if (candidate == null || candidate.id == null || !candidate.id.matches("[a-f0-9]{32}")) continue;
            Entry e = read(file(candidate.id));
            if (e == null || LIVE.contains(e.id)) continue;
            Info info = null;
            try {
                if (e.complete) { forget(e); continue; }
                info = locate(resolver, e);
                if (Thread.currentThread().isInterrupted()) { result.add(new Recovered(e, info)); break; }
                if (e.publishPending) { result.add(new Recovered(e, info)); continue; }
                if (info == null) { forget(e); continue; }
                if (owned(e, info.name) && DocumentsContract.deleteDocument(resolver, info.uri)) { forget(e); continue; }
                result.add(new Recovered(e, info));
            } catch (Exception unavailable) { result.add(new Recovered(e, info)); }
        }
        return result;
    }

    Entry change(ContentResolver resolver, String id, String version, boolean keep) throws IOException {
        if (!id.matches("[a-f0-9]{32}") || LIVE.contains(id)) throw new TransferFailure(R.string.ui_previous_pc_tasks_are_stopping_try_again_shortly);
        checkCancelled(Thread.currentThread()::isInterrupted);
        Entry e = read(file(id));
        if (e == null) return null;
        if (!keep) {
            Info info = locate(resolver, e);
            if (info != null) {
                if (!info.version().equals(version)) throw new TransferFailure(R.string.ui_unfinished_download_changed);
                checkCancelled(Thread.currentThread()::isInterrupted);
                if (!DocumentsContract.deleteDocument(resolver, info.uri)) throw new TransferFailure(R.string.ui_unfinished_download_remove_failed);
            }
        }
        checkCancelled(Thread.currentThread()::isInterrupted);
        forget(e);
        return e;
    }

    static void checkCancelled(BooleanSupplier cancelled) throws TransferFailure {
        if (cancelled.getAsBoolean()) throw new TransferFailure(R.string.ui_transfer_canceled);
    }
    static final class Publication {
        boolean started;
        <T> T run(BooleanSupplier cancelled, Callable<T> mutation) throws IOException {
            checkCancelled(cancelled);
            started = true;
            try { return mutation.call(); }
            catch (IOException error) { throw error; }
            catch (RuntimeException error) { throw error; }
            catch (Exception error) { throw new IOException("Could not publish download", error); }
        }
    }

    final class Target {
        private final Entry entry;
        private ContentResolver resolver;
        private Uri parent;
        private boolean published, commitRecorded;
        Target(Entry entry) { this.entry = entry; }
        OutputStream open(ContentResolver resolver, Uri parent, String mime) throws IOException { return open(resolver, parent, mime, () -> false); }
        OutputStream open(ContentResolver resolver, Uri parent, String mime, BooleanSupplier cancelled) throws IOException {
            this.resolver = resolver; this.parent = parent;
            checkCancelled(cancelled);
            Uri created = DocumentsContract.createDocument(resolver, parent, mime, entry.name);
            if (created == null) throw new FileNotFoundException("Could not create download");
            entry.uri = created.toString(); save(entry);
            checkCancelled(cancelled);
            Info info = inspect(resolver, created);
            if (info == null) throw new FileNotFoundException("Created download unavailable");
            entry.storedName = info.name; save(entry);
            if ((info.flags & Document.FLAG_SUPPORTS_RENAME) != 0) {
                entry.plannedName = ".plumelink-" + entry.id + ".part";
                int dot = entry.name.lastIndexOf('.');
                if (dot > 0 && entry.name.substring(dot + 1).matches("[A-Za-z0-9]{1,16}")) entry.plannedName += entry.name.substring(dot);
                save(entry);
                try {
                    checkCancelled(cancelled);
                    Uri moved = DocumentsContract.renameDocument(resolver, created, entry.plannedName);
                    if (moved != null) { entry.uri = moved.toString(); save(entry); }
                    checkCancelled(cancelled);
                    Info renamed = inspect(resolver, moved == null ? created : moved);
                    if (renamed == null) throw new FileNotFoundException("Renamed download unavailable");
                    entry.temporary = !Objects.equals(entry.storedName, renamed.name);
                    entry.uri = renamed.uri.toString(); entry.storedName = renamed.name; save(entry);
                } catch (UnsupportedOperationException ignored) { entry.plannedName = ""; save(entry); }
            }
            checkCancelled(cancelled);
            OutputStream stream = resolver.openOutputStream(Uri.parse(entry.uri), "w");
            if (stream == null) throw new FileNotFoundException("Could not open download");
            return stream;
        }
        void publish() throws IOException { publish(() -> false); }
        void publish(BooleanSupplier cancelled) throws IOException {
            if (published) return;
            checkCancelled(cancelled);
            if (entry.temporary) {
                String name = uniqueName(resolver, parent, entry.name);
                checkCancelled(cancelled);
                entry.publishPending = true; save(entry);
                Publication publication = new Publication();
                Uri result;
                try {
                    result = publication.run(cancelled, () -> DocumentsContract.renameDocument(resolver, Uri.parse(entry.uri), name));
                } catch (IOException | RuntimeException error) {
                    if (!publication.started) { entry.publishPending = false; save(entry); }
                    throw error;
                }
                if (result == null) throw new IOException("Could not publish download");
                published = true;
                entry.uri = result.toString(); save(entry);
                Info info = inspect(resolver, result);
                if (info == null || Objects.equals(info.name, entry.storedName) || owned(entry, info.name))
                    throw new IOException("Download name was not published");
            }
            published = true;
            entry.complete = true; entry.publishPending = false; save(entry); forget(entry);
            commitRecorded = true;
        }
        boolean outcomeUnconfirmed() { return entry.publishPending || published && !commitRecorded; }

        void abort() {
            if (published || entry.publishPending) return;
            try {
                Info info = resolver == null ? null : locate(resolver, entry);
                if (info == null || (Objects.equals(info.name, entry.storedName) || owned(entry, info.name))
                    && DocumentsContract.deleteDocument(resolver, info.uri)) forget(entry);
            } catch (Exception ignored) {}
        }
        void release() { LIVE.remove(entry.id); }
    }

    static String numberedName(String name, int number) {
        if (number == 0) return name;
        int dot = name.lastIndexOf('.');
        String extension = dot > 0 && name.length() - dot <= 33 ? name.substring(dot) : "";
        String stem = extension.isEmpty() ? name : name.substring(0, dot);
        String suffix = " (" + number + ")";
        int budget = 240 - (suffix + extension).getBytes(StandardCharsets.UTF_8).length;
        StringBuilder prefix = new StringBuilder();
        int bytes = 0;
        for (int i = 0; i < stem.length();) {
            int code = stem.codePointAt(i);
            String part = new String(Character.toChars(code));
            bytes += part.getBytes(StandardCharsets.UTF_8).length;
            if (bytes > budget) break;
            prefix.append(part); i += Character.charCount(code);
        }
        return prefix + suffix + extension;
    }
    private static String uniqueName(ContentResolver resolver, Uri parent, String name) throws IOException {
        BitSet used = new BitSet();
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(parent, DocumentsContract.getDocumentId(parent));
        try (Cursor cursor = resolver.query(children, new String[]{Document.COLUMN_DISPLAY_NAME}, null, null, null)) {
            if (cursor == null) throw new IOException("Document provider unavailable");
            while (cursor.moveToNext()) {
                String existing = cursor.getString(0);
                if (existing.equalsIgnoreCase(name)) { used.set(0); continue; }
                int end = existing.lastIndexOf(')'), start = existing.lastIndexOf(" (");
                if (start < 0 || end <= start + 2 || end - start > 9) continue;
                try {
                    int number = Integer.parseInt(existing.substring(start + 2, end));
                    if (number > 0 && number <= 1_000_000 && existing.equalsIgnoreCase(numberedName(name, number))) used.set(number);
                } catch (NumberFormatException ignored) {}
            }
        } catch (RuntimeException error) { throw new IOException("Could not check destination names", error); }
        int number = used.nextClearBit(0);
        if (number > 1_000_000) throw new IOException("No available download name");
        return numberedName(name, number);
    }
}
