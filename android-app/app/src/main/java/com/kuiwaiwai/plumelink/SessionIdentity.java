package com.kuiwaiwai.plumelink;

final class SessionIdentity {
    private SessionIdentity() {}
    static String owner(String endpoint, String identity, String pin) {
        return EndpointUtils.sha256Hex((identity.matches("[a-f0-9]{24}") ? identity : endpoint) + "\n" + pin);
    }
    static boolean accepts(String source, String current) { return !source.isEmpty() && source.equals(current); }
    static String transport(String endpoint, String owner, String usbKey) {
        String access = EndpointUtils.isUsbEndpoint(endpoint) && usbKey != null ? usbKey : "";
        return EndpointUtils.sha256Hex(owner + "\n" + endpoint + "\n" + access);
    }
}
