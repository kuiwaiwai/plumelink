package com.kuiwaiwai.plumelink;

import org.json.JSONArray;
import org.junit.Test;
import static org.junit.Assert.*;

public final class UsbAccessTest {
    @Test public void onlyExactLoopbackOriginsAndKeysAreStored() {
        String endpoint="https://127.0.0.1:17840",key="a".repeat(64);
        String saved=UsbAccess.put("",endpoint,key);assertEquals(key,UsbAccess.get(saved,endpoint));
        for(String invalid:new String[]{"http://127.0.0.1:17840","https://127.0.0.2:17840","https://127.0.0.1:17840/path","https://127.0.0.1:17840?x=1","https://user@127.0.0.1:17840","https://127.0.0.1:17840#fragment"}) {
            assertEquals(saved,UsbAccess.put(saved,invalid,key));assertEquals("",UsbAccess.get(saved,invalid));
        }
        assertEquals(saved,UsbAccess.put(saved,endpoint,"A".repeat(64)));assertEquals("",UsbAccess.get(UsbAccess.put(saved,endpoint,""),endpoint));
        assertTrue(UsbAccess.matchesTag(key,UsbAccess.tag(key)));assertFalse(UsbAccess.matchesTag("b".repeat(64),UsbAccess.tag(key)));assertFalse(UsbAccess.matchesTag(key,""));
    }
    @Test public void cacheEvictionAndReplacementAreBounded() throws Exception {
        String saved="";for(int i=0;i<UsbAccess.LIMIT+3;i++)saved=UsbAccess.put(saved,"https://127.0.0.1:"+(20000+i),"a".repeat(64));
        assertEquals(UsbAccess.LIMIT,new JSONArray(saved).length());assertEquals("",UsbAccess.get(saved,"https://127.0.0.1:20000"));
        saved=UsbAccess.put(saved,"https://127.0.0.1:20130","b".repeat(64));assertTrue(new JSONArray(saved).length()<=UsbAccess.LIMIT);assertEquals("b".repeat(64),UsbAccess.get(saved,"https://127.0.0.1:20130"));
        assertEquals("",UsbAccess.get("x".repeat(32769),"https://127.0.0.1:20130"));
    }
    @Test public void pairingLinksCannotMixUsbCredentialsIntoWirelessOrAmbiguousParameters() {
        String base="plumelink://connect?address=127.0.0.1:17840",key="a".repeat(64);
        assertEquals(key,PairingLink.parse(base+"&usb="+key).usbAccess);
        assertNull(PairingLink.parse(base+"&usb="+key+"&usb="+key));assertNull(PairingLink.parse(base+"&usb="));
        assertNull(PairingLink.parse("plumelink://connect?address=192.168.1.2:17840&usb="+key));
    }
}
