package com.kuiwaiwai.plumelink;

import org.junit.Test;
import static org.junit.Assert.*;

public final class PairingLinkTest {
    @Test public void preservesLeadingZerosAndLegacyLinks() {
        PairingLink link = PairingLink.parse("plumelink://connect?address=192.168.1.2%3A19827&code=012345");
        assertNotNull(link); assertEquals("012345", link.code); assertEquals("192.168.1.2:19827", link.address);
        assertEquals("", PairingLink.parse("plumelink://connect?address=192.168.1.2:19827").code);
        assertNotNull(PairingLink.parse("plumelink://connect?address=%5Bfd00%3A%3A1%5D%3A19827&code=000001"));
    }
    @Test public void keepsThePortAdvertisedByThePc() {
        for (int port : new int[] {443, 29827, 65535}) {
            PairingLink link = PairingLink.parse("plumelink://connect?address=192.168.1.2%3A" + port + "&code=012345");
            assertNotNull(link);
            assertEquals("192.168.1.2:" + port, link.address);
        }
    }
    @Test public void rejectsMalformedAndAmbiguousData() {
        String base = "plumelink://connect?address=192.168.1.2:19827";
        for (String value : new String[] {base + "&code=12345", base + "&code=１２３４５６", base + "&code=", base + "&code=123456&code=654321", base + "&address=10.0.0.1:80", base + "#fragment", base + "&other=value", "https://connect?address=192.168.1.2:80", "plumelink://user@connect?address=192.168.1.2:80", "plumelink://connect/path?address=192.168.1.2:80", "plumelink://connect?address=example.com:80", "plumelink://connect?address=192.168.1.2:0", "plumelink://connect?address=192.168.1.2:65536"}) {
            assertNull(value, PairingLink.parse(value));
        }
    }
}
