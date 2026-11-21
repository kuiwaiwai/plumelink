package com.kuiwaiwai.plumelink;

import android.content.ContentResolver;
import android.content.UriPermission;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;

final class ReceiveDirectory {
    private ReceiveDirectory() {}

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
