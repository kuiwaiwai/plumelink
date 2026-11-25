package com.kuiwaiwai.plumelink;

import java.util.HashSet;
import java.util.Set;

final class LocalNetworks<T> {
    private final Set<T> available = new HashSet<>();
    void add(T network) { available.add(network); }
    boolean removeLast(T network) { return available.remove(network) && available.isEmpty(); }
    void clear() { available.clear(); }
}
