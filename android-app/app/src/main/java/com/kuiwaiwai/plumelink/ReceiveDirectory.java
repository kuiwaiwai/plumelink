package com.kuiwaiwai.plumelink;

import android.content.ContentResolver;
import android.content.Intent;
import android.content.UriPermission;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import java.util.*;
import java.util.function.Supplier;

final class ReceiveDirectory {
    private ReceiveDirectory() {}
    static String labelPath(String authority, String documentId, String internalStorage, String externalStorage) {
        if (!"com.android.externalstorage.documents".equals(authority)) return documentId;
        int separator = documentId.indexOf(':');
        if (separator < 0) return documentId;
        String relative = documentId.substring(separator + 1);
        if (!relative.isEmpty()) return relative;
        return "primary".equalsIgnoreCase(documentId.substring(0, separator)) ? internalStorage : externalStorage;
    }
    private static final Object ACCESS = new Object();
    private static final Map<String, Integer> active = new HashMap<>();
    private static long revision;

    static final class Access implements AutoCloseable {
        private final String tree;
        private boolean closed;
        Access(String tree) { this.tree = tree; }
        @Override public void close() {
            synchronized (ACCESS) {
                if (closed) return;
                closed = true;
                active.computeIfPresent(tree, (key, count) -> count == 1 ? null : count - 1);
                revision++;
            }
        }
    }
    static Access retain(String tree) {
        synchronized (ACCESS) {
            active.merge(tree, 1, Integer::sum); revision++;
            return new Access(tree);
        }
    }
    static void select(ContentResolver resolver, ComputerStore store, Uri tree, int offered) {
        int required = Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION;
        if (!DocumentsContract.isTreeUri(tree) || (offered & required) != required)
            throw new SecurityException("Read and write directory access required");
        synchronized (ACCESS) {
            resolver.takePersistableUriPermission(tree, offered & required);
            store.setReceiveTree(tree.toString()); revision++;
        }
    }
    static void releaseUnused(ContentResolver resolver, Supplier<String> selected, PendingDownloads pending) {
        long checked;
        synchronized (ACCESS) { checked = revision; }
        Set<String> keep = pending.requiredTrees();
        if (keep == null) return;
        synchronized (ACCESS) {
            if (checked != revision) return;
            keep.add(selected.get()); keep.addAll(active.keySet());
            try {
                for (UriPermission permission : resolver.getPersistedUriPermissions()) {
                    Uri tree = permission.getUri();
                    if (!DocumentsContract.isTreeUri(tree) || keep.contains(tree.toString())) continue;
                    int flags = (permission.isReadPermission() ? Intent.FLAG_GRANT_READ_URI_PERMISSION : 0)
                        | (permission.isWritePermission() ? Intent.FLAG_GRANT_WRITE_URI_PERMISSION : 0);
                    if (flags != 0) try { resolver.releasePersistableUriPermission(tree, flags); }
                    catch (RuntimeException ignored) {}
                }
            } catch (RuntimeException ignored) {}
        }
    }

    static Uri writableDocument(ContentResolver resolver, String value) {
        try {
            Uri tree = Uri.parse(value);
            if (!DocumentsContract.isTreeUri(tree)) return null;
            boolean writable = false;
            for (UriPermission permission : resolver.getPersistedUriPermissions()) {
                if (tree.equals(permission.getUri()) && permission.isReadPermission() && permission.isWritePermission()) {
                    writable = true;
                    break;
                }
            }
            if (!writable) return null;
            Uri document = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree));
            String[] columns = {Document.COLUMN_MIME_TYPE, Document.COLUMN_FLAGS};
            try (Cursor cursor = resolver.query(document, columns, null, null, null)) {
                if (cursor != null && cursor.moveToFirst() && Document.MIME_TYPE_DIR.equals(cursor.getString(0))
                    && (cursor.getInt(1) & Document.FLAG_DIR_SUPPORTS_CREATE) != 0) return document;
            }
        } catch (RuntimeException ignored) {
            // A document provider can be unavailable until its storage is mounted again.
        }
        return null;
    }
}
