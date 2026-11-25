package com.kuiwaiwai.plumelink;

final class ConnectionIdentity {
    enum Decision { ACCEPT, CONFIRM, CERTIFICATE_CHANGED, IDENTITY_CHANGED }
    private ConnectionIdentity() {}
    static Decision decide(String hint, String reported, String pin, String expected, String current, boolean recognizedCertificate) {
        if (!hint.isEmpty() && !reported.isEmpty() && !hint.equals(reported)) return Decision.IDENTITY_CHANGED;
        if (pin.isEmpty() || (!expected.isEmpty() && !expected.equals(pin)) || (!current.isEmpty() && !current.equals(pin))) return Decision.CERTIFICATE_CHANGED;
        return current.equals(pin) || recognizedCertificate ? Decision.ACCEPT : Decision.CONFIRM;
    }
}
