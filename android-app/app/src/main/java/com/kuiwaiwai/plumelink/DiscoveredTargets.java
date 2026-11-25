package com.kuiwaiwai.plumelink;

import java.util.Map;
import java.util.function.Function;

final class DiscoveredTargets {
    private DiscoveredTargets() {}
    static String choose(String saved, String identity, Map<String, String> nearby, Function<String,String> identityFor) {
        if (!identity.matches("[a-f0-9]{24}")) return saved;
        if (nearby.containsKey(saved) && identity.equals(identityFor.apply(saved))) return saved;
        for (String candidate : nearby.keySet()) if (identity.equals(identityFor.apply(candidate))) return candidate;
        return saved;
    }
}
