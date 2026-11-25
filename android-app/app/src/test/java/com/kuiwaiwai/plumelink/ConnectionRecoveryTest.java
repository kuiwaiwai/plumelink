package com.kuiwaiwai.plumelink;

import org.junit.Test;
import java.util.*;
import static org.junit.Assert.*;

public class ConnectionRecoveryTest {
    @Test public void discoveryClaimsNeverReplaceCertificateVerificationOrPermitAnotherComputer() {
        assertEquals(ConnectionIdentity.Decision.CERTIFICATE_CHANGED, ConnectionIdentity.decide("a", "a", "new", "old", "old", true));
        assertEquals(ConnectionIdentity.Decision.IDENTITY_CHANGED, ConnectionIdentity.decide("a", "b", "same", "same", "same", true));
        assertEquals(ConnectionIdentity.Decision.CONFIRM, ConnectionIdentity.decide("a", "a", "new", "", "", false));
        assertEquals(ConnectionIdentity.Decision.CERTIFICATE_CHANGED, ConnectionIdentity.decide("", "", "", "", "", false));
    }
    @Test public void recognizedCertificateCanRestoreAManualAddressWithoutDiscoveryButCannotOverrideAPinMismatch() {
        assertEquals(ConnectionIdentity.Decision.ACCEPT, ConnectionIdentity.decide("", "a", "same", "", "", true));
        assertEquals(ConnectionIdentity.Decision.ACCEPT, ConnectionIdentity.decide("a", "", "same", "same", "same", false));
        assertEquals(ConnectionIdentity.Decision.CERTIFICATE_CHANGED, ConnectionIdentity.decide("", "a", "same", "different", "", true));
    }
    @Test public void droppingOneLocalNetworkDoesNotReportOfflineWhileAnotherRemains() {
        LocalNetworks<String> networks = new LocalNetworks<>();
        networks.add("wifi");networks.add("vpn");networks.add("wifi");
        assertFalse(networks.removeLast("vpn"));assertFalse(networks.removeLast("unknown"));
        assertTrue(networks.removeLast("wifi"));assertFalse(networks.removeLast("wifi"));
        networks.add("ethernet");networks.clear();assertFalse(networks.removeLast("ethernet"));
    }
    @Test public void choosesOnlyTheMatchingDiscoveredIdentityAndPrefersTheSavedAddressWhenStillPresent() {
        Map<String,String> nearby = new LinkedHashMap<>();nearby.put("first","one");nearby.put("next","two");
        Map<String,String> ids = Map.of("first","a".repeat(24),"next","b".repeat(24));
        assertEquals("next",DiscoveredTargets.choose("old","b".repeat(24),nearby,ids::get));
        assertEquals("first",DiscoveredTargets.choose("first","a".repeat(24),nearby,ids::get));
        assertEquals("old",DiscoveredTargets.choose("old","",nearby,ids::get));
        assertEquals("old",DiscoveredTargets.choose("old","c".repeat(24),nearby,ids::get));
    }
    @Test public void parsesOnlyTheBoundedPlumeLinkIdentityAndCancellationPreventsANewProbe() {
        String id="a".repeat(24);
        assertEquals(id,ConnectionProbe.parseIdentity("{\"service\":\"plumelink\",\"version\":1,\"id\":\""+id+"\"}"));
        for(String text:new String[]{"", "[]", "{}", "x".repeat(1025), "{\"service\":\"other\",\"version\":1,\"id\":\""+id+"\"}", "{\"service\":\"plumelink\",\"version\":\"1\",\"id\":\""+id+"\"}"}) assertEquals("",ConnectionProbe.parseIdentity(text));
        ConnectionProbe probe=new ConnectionProbe();probe.cancel();assertEquals(ConnectionProbe.State.CANCELLED,probe.inspect("https://127.0.0.1:17840").state());
    }
}
