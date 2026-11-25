package com.kuiwaiwai.plumelink;
import org.junit.Test;
import static org.junit.Assert.*;

public class SessionIdentityTest {
    @Test public void ownershipFollowsVerifiedIdentityAcrossTransportsButNotAnotherPcOrCertificate() {
        String id="a".repeat(24),pin="c".repeat(64),usb="https://127.0.0.1:49821";
        String first=SessionIdentity.owner("https://192.168.1.2:17840",id,pin);
        assertEquals(first,SessionIdentity.owner(usb,id,pin));
        assertFalse(SessionIdentity.accepts(first,SessionIdentity.owner(usb,"b".repeat(24),pin)));
        assertFalse(SessionIdentity.accepts(first,SessionIdentity.owner(usb,id,"d".repeat(64))));
        assertFalse(SessionIdentity.accepts("",first));
    }
    @Test public void recoveredTransferKeepsItsOwner() throws Exception {
        java.io.File folder=java.nio.file.Files.createTempDirectory("transfer-owner").toFile();
        try {
            TransferJournal journal=new TransferJournal(folder);String owner=SessionIdentity.owner("https://127.0.0.1:49821","a".repeat(24),"pin");
            String id="e".repeat(32);
            java.nio.file.Files.write(new java.io.File(folder,"transfer.active").toPath(),new org.json.JSONObject().put("id",id).put("origin","https://127.0.0.1:49821").put("owner",owner).toString().getBytes(java.nio.charset.StandardCharsets.UTF_8));
            assertEquals(owner,journal.recover().owner);assertTrue(journal.acknowledge(id));
            try(TransferJournal.Lease lease=journal.begin("https://127.0.0.1:49821",owner)){assertNull(journal.recover());assertEquals(owner,lease.owner());}
        } finally { try(java.util.stream.Stream<java.nio.file.Path> paths=java.nio.file.Files.walk(folder.toPath())){for(java.nio.file.Path path:paths.sorted(java.util.Comparator.reverseOrder()).toArray(java.nio.file.Path[]::new))java.nio.file.Files.deleteIfExists(path);} }
    }
    @Test public void rotatingUsbAccessChangesOnlyTheActiveTransport() {
        String endpoint="https://127.0.0.1:49821",id="a".repeat(24),pin="c".repeat(64);
        String owner=SessionIdentity.owner(endpoint,id,pin);
        String first=SessionIdentity.transport(endpoint,owner,"1".repeat(64));
        assertEquals(first,SessionIdentity.transport(endpoint,owner,"1".repeat(64)));
        assertFalse(SessionIdentity.accepts(first,SessionIdentity.transport(endpoint,owner,"2".repeat(64))));
        assertFalse(SessionIdentity.accepts(first,SessionIdentity.transport("https://127.0.0.1:49822",owner,"1".repeat(64))));
        assertEquals(owner,SessionIdentity.owner("https://192.168.1.2:17840",id,pin));
        assertFalse(first.contains("1".repeat(64)));
    }
    @Test public void wirelessTransportNeverDependsOnUsbAccess() {
        String endpoint="https://192.168.1.2:17840",owner=SessionIdentity.owner(endpoint,"a".repeat(24),"pin");
        assertEquals(SessionIdentity.transport(endpoint,owner,""),SessionIdentity.transport(endpoint,owner,"1".repeat(64)));
    }
}
