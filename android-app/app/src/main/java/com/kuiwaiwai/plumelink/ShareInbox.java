package com.kuiwaiwai.plumelink;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.UUID;

final class ShareInbox {
    static final int LIMIT = 16;
    record File(String id, String uri, String name, long size, int error, boolean pending) {}
    private final LinkedHashMap<String, File> files = new LinkedHashMap<>();
    private String revision = nextId();

    private static String nextId() { return UUID.randomUUID().toString().replace("-", ""); }
    String revision() { return revision; }
    List<File> files() { return java.util.Collections.unmodifiableList(new ArrayList<>(files.values())); }
    boolean empty() { return files.isEmpty(); }
    boolean ready() { return !empty() && files.values().stream().noneMatch(file -> file.pending || file.error != 0); }

    boolean add(List<String> uris) {
        LinkedHashMap<String, File> merged = new LinkedHashMap<>(files);
        for (String uri : uris) {
            if (uri == null || uri.length() > 8192 || !uri.startsWith("content://")) return false;
            merged.computeIfAbsent(uri, key -> new File(nextId(), key, "", -1, 0, true));
            if (merged.size() > LIMIT) return false;
        }
        if (merged.size() != files.size()) {
            files.clear(); files.putAll(merged); revision = nextId();
        }
        return true;
    }
    void remove(String expected, String id) {
        if (!revision.equals(expected)) return;
        if (files.values().removeIf(file -> file.id.equals(id))) revision = nextId();
    }
    void clear(String expected) {
        if (!revision.equals(expected)) return;
        files.clear(); revision = nextId();
    }
    boolean consume(String expected) {
        if (!revision.equals(expected) || !ready()) return false;
        clear(expected); return true;
    }
    File retry(String expected, String id) {
        if (!revision.equals(expected)) return null;
        for (java.util.Map.Entry<String, File> entry : files.entrySet()) {
            File current = entry.getValue();
            if (!current.id.equals(id) || current.pending || current.error == 0) continue;
            File retried = new File(nextId(), current.uri, current.name, current.size, 0, true);
            entry.setValue(retried); revision = nextId();
            return retried;
        }
        return null;
    }
    void metadata(File source, String name, long size, int error) {
        File current = files.get(source.uri);
        if (current == null || !current.id.equals(source.id) || !current.pending) return;
        files.put(source.uri, new File(source.id, source.uri, name, size, error, false));
        revision = nextId();
    }
    ArrayList<String> uris() { return new ArrayList<>(files.keySet()); }
}
