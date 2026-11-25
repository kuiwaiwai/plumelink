package com.kuiwaiwai.plumelink;

import android.content.ClipData;
import android.content.Intent;
import android.net.Uri;
import android.os.Parcelable;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;

final class ShareIntents {
    private ShareIntents() {}
    static boolean matches(Intent intent) {
        return intent != null && (Intent.ACTION_SEND.equals(intent.getAction()) || Intent.ACTION_SEND_MULTIPLE.equals(intent.getAction()));
    }
    static List<String> files(Intent intent, boolean external) {
        if (intent == null || (external && !matches(intent))) throw new IllegalArgumentException();
        LinkedHashSet<String> uris = new LinkedHashSet<>();
        if (external) {
            if (Intent.ACTION_SEND_MULTIPLE.equals(intent.getAction())) {
                ArrayList<Parcelable> streams = intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM);
                if (streams != null) {
                    if (streams.size() > ShareInbox.LIMIT) throw new IllegalArgumentException();
                    for (Parcelable item : streams) add(uris, item);
                }
            } else if (intent.hasExtra(Intent.EXTRA_STREAM)) add(uris, intent.getParcelableExtra(Intent.EXTRA_STREAM));
        }
        ClipData clip = intent.getClipData();
        if (clip != null) {
            if (clip.getItemCount() > ShareInbox.LIMIT) throw new IllegalArgumentException();
            for (int i = 0; i < clip.getItemCount(); i++) {
                ClipData.Item item = clip.getItemAt(i);
                if (item.getUri() != null) add(uris, item.getUri());
                else if (item.getIntent() != null) throw new IllegalArgumentException();
            }
        } else if (!external && intent.getData() != null) add(uris, intent.getData());
        if (uris.isEmpty()) throw new IllegalArgumentException();
        return java.util.Collections.unmodifiableList(new ArrayList<>(uris));
    }
    private static void add(LinkedHashSet<String> uris, Object value) {
        if (!(value instanceof Uri uri) || !"content".equals(uri.getScheme()) || uri.getAuthority() == null
            || uri.getAuthority().isEmpty() || uri.toString().length() > 8192) throw new IllegalArgumentException();
        uris.add(uri.toString());
        if (uris.size() > ShareInbox.LIMIT) throw new IllegalArgumentException();
    }
}
