package com.kuiwaiwai.plumelink;

import android.content.*;
import org.json.*;
import org.junit.Test;
import java.lang.reflect.Proxy;
import java.util.*;
import static org.junit.Assert.*;

public class ComputerPreferencesTest {
    private static final String FIRST = "https://192.168.1.2:17840", NEXT = "https://192.168.1.3:17841", PC = "a".repeat(24), OTHER = "b".repeat(24);
    private static final class Memory {
        final Map<String,Object> values = new HashMap<>();
        final Map<String,Object> pending = new HashMap<>();
        final SharedPreferences.Editor editor = (SharedPreferences.Editor)Proxy.newProxyInstance(getClass().getClassLoader(),new Class<?>[]{SharedPreferences.Editor.class},(proxy,method,args)->{
            String name=method.getName();
            if (name.startsWith("put")) { pending.put((String)args[0], args[1]); return proxy; }
            if (name.equals("remove")) { pending.put((String)args[0], null); return proxy; }
            if (name.equals("apply") || name.equals("commit")) { pending.forEach((key,value)->{if(value==null)values.remove(key);else values.put(key,value);});pending.clear();return name.equals("commit")?true:null; }
            return null;
        });
        final SharedPreferences preferences = (SharedPreferences)Proxy.newProxyInstance(getClass().getClassLoader(),new Class<?>[]{SharedPreferences.class},(proxy,method,args)->{
            String name=method.getName();
            if(name.equals("edit"))return editor;
            if(name.equals("getAll"))return new HashMap<>(values);
            if(name.equals("contains"))return values.containsKey(args[0]);
            if(name.startsWith("get"))return values.getOrDefault(args[0],args[1]);
            return null;
        });
        final Context context = new ContextWrapper(null) {
            @Override public Context getApplicationContext(){return this;}
            @Override public SharedPreferences getSharedPreferences(String name,int mode){return preferences;}
        };
        final ComputerStore store = new ComputerStore(context);
        void old(String url,String value){values.put("mobile_settings_"+EndpointUtils.sha256Hex(url),value);}
        void bind(String url,String identity){store.bindTrustedIdentity(url,java.net.URI.create(url).getHost(),identity,"c".repeat(64));}
    }
    private String snapshot(String name)throws Exception {
        return new JSONObject(MobilePreferencesTest.sample()).put("shortcutPlans",new JSONArray().put(ShortcutPlansTest.plan("d",name))).toString();
    }
    @Test public void replacingOnePcTrustClearsBothTransportCredentialsButKeepsOtherPcs() {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.rememberComputer(FIRST);
        String usb="https://127.0.0.1:54321";m.store.bindTrustedIdentity(usb,"127.0.0.1",PC,"c".repeat(64));m.store.rememberComputer(usb);
        m.store.setToken(PC,"old-token");m.store.setDeviceId(PC,"old-device");
        m.bind(NEXT,OTHER);m.store.rememberComputer(NEXT);m.store.setToken(OTHER,"other-token");
        m.store.prepareTrustReplacement(usb,PC,PC);
        assertEquals("",m.store.token(PC));assertEquals("",m.store.token(usb));assertEquals("",m.store.token(FIRST));assertEquals("",m.store.deviceId(PC));
        assertEquals("other-token",m.store.token(OTHER));assertEquals("c".repeat(64),m.store.trustedPin("192.168.1.3",OTHER));
    }
    @Test public void reinstallingThePcRemovesItsOldEntryWithoutReusingItsCredentials() {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.rememberComputer(FIRST);m.store.setToken(PC,"old-token");m.store.setDeviceId(PC,"old-device");
        m.store.prepareTrustReplacement(FIRST,PC,OTHER);
        m.store.bindTrustedIdentity(FIRST,"192.168.1.2",OTHER,"d".repeat(64));m.store.rememberComputer(FIRST);
        assertEquals("",m.store.token(OTHER));assertEquals("",m.store.deviceId(OTHER));assertEquals("",m.store.token(PC));
        assertEquals(OTHER,m.store.identityForUrl(FIRST));assertEquals("d".repeat(64),m.store.trustedPin("192.168.1.2",OTHER));
    }
    @Test public void discoveryOfANewIdentityStillClearsThePreviousEndpointOwner() {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.rememberComputer(FIRST);m.store.setToken(PC,"old-token");
        m.store.prepareTrustReplacement(FIRST,OTHER,OTHER);
        assertEquals("",m.store.token(PC));assertEquals("",m.store.token(FIRST));
        m.store.bindTrustedIdentity(FIRST,"192.168.1.2",OTHER,"d".repeat(64));
        assertEquals("",m.store.token(OTHER));
    }
    @Test public void migratesPreviousAddressSettingsAndThenFollowsTheTrustedComputer()throws Exception {
        Memory m=new Memory();m.old(FIRST,MobilePreferencesTest.sampleV2());m.bind(FIRST,PC);
        assertEquals("voice",new JSONObject(m.store.mobilePreferences(FIRST)).getString("microphoneMode"));
        m.store.setMobilePreferences(FIRST,snapshot("Work"));m.bind(NEXT,PC);
        assertEquals("Work",new JSONObject(m.store.mobilePreferences(NEXT)).getJSONArray("shortcutPlans").getJSONObject(0).getString("name"));
        assertFalse(m.values.containsKey("mobile_settings_"+EndpointUtils.sha256Hex(FIRST)));
    }
    @Test public void anotherComputerAtTheSameAddressDoesNotInheritPlansOrOverwriteTheOriginal()throws Exception {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.setMobilePreferences(FIRST,snapshot("First"));m.bind(FIRST,OTHER);
        assertEquals("",m.store.mobilePreferences(FIRST));m.store.setMobilePreferences(FIRST,snapshot("Other"));m.bind(NEXT,PC);
        assertEquals("First",new JSONObject(m.store.mobilePreferences(NEXT)).getJSONArray("shortcutPlans").getJSONObject(0).getString("name"));
        assertEquals("Other",new JSONObject(m.store.mobilePreferences(FIRST)).getJSONArray("shortcutPlans").getJSONObject(0).getString("name"));
    }
    @Test public void keepsCanonicalSettingsOverStaleAddressCopiesAndRejectsOldWriters()throws Exception {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.setMobilePreferences(FIRST,snapshot("Canonical"));m.old(FIRST,snapshot("Stale"));m.bind(NEXT,PC);
        m.store.setMobilePreferences(NEXT,MobilePreferencesTest.sampleV2());m.store.setMobilePreferences(NEXT,"{}");
        assertEquals("Canonical",new JSONObject(m.store.mobilePreferences(NEXT)).getJSONArray("shortcutPlans").getJSONObject(0).getString("name"));
    }
    @Test public void deletingTheSavedComputerRemovesItsPlans()throws Exception {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.setMobilePreferences(FIRST,snapshot("Work"));m.store.rememberComputer(FIRST);
        m.store.forgetComputer(FIRST);m.bind(NEXT,PC);assertEquals("",m.store.mobilePreferences(NEXT));
    }
    @Test public void matchesOnlyAnUnambiguousTrustedCertificate() {
        Memory m=new Memory();m.bind(FIRST,PC);
        assertEquals(PC,m.store.trustedIdentity("c".repeat(64)));
        assertEquals("",m.store.trustedIdentity("unknown"));
        m.bind(NEXT,OTHER);assertEquals("",m.store.trustedIdentity("c".repeat(64)));
    }
    @Test public void aVerifiedIdentityMigratesTheUniqueLegacyPairingAtAnOldAddress()throws Exception {
        Memory m=new Memory();m.values.put("pin_192.168.1.2","c".repeat(64));
        m.store.rememberComputer(FIRST);m.store.setMobilePreferences(FIRST,snapshot("Legacy"));m.store.setToken(FIRST,"a".repeat(64));
        m.bind(NEXT,PC);
        assertEquals("a".repeat(64),m.store.token(PC));assertEquals("",m.store.token(FIRST));
        assertEquals("Legacy",new JSONObject(m.store.mobilePreferences(NEXT)).getJSONArray("shortcutPlans").getJSONObject(0).getString("name"));
        assertEquals(List.of(NEXT),m.store.visibleSavedComputers());
    }
    @Test public void usbKeepsTheWirelessEndpointAndSharesOnlyTheVerifiedComputerPreferences()throws Exception {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.rememberComputer(FIRST);m.store.setToken(PC,"a".repeat(64));m.store.setMobilePreferences(PC,snapshot("Work"));
        String usb="https://127.0.0.1:49821";m.bind(usb,PC);m.store.rememberComputer(usb);
        assertEquals(FIRST,m.store.wirelessEndpoint(PC));assertEquals(usb,m.store.endpointFor(PC));assertEquals(List.of(PC),m.store.visibleSavedRecords());
        assertEquals("a".repeat(64),m.store.token(PC));assertEquals("Work",new JSONObject(m.store.mobilePreferences(PC)).getJSONArray("shortcutPlans").getJSONObject(0).getString("name"));
        m.store.preferUsb(PC,false);assertEquals(FIRST,m.store.endpointFor(PC));
    }
    @Test public void reusedUsbPortDoesNotMovePairingSettingsOrForgetTheOtherComputer()throws Exception {
        Memory m=new Memory();String usb="https://127.0.0.1:49821";
        m.bind(usb,PC);m.store.rememberComputer(usb);m.store.setToken(PC,"a".repeat(64));m.store.setMobilePreferences(PC,snapshot("Private"));
        m.store.bindTrustedIdentity(usb,"127.0.0.1",OTHER,"d".repeat(64));m.store.rememberComputer(usb);
        assertEquals(Set.of(PC,OTHER),new HashSet<>(m.store.visibleSavedRecords()));assertEquals("",m.store.token(OTHER));assertEquals("",m.store.mobilePreferences(OTHER));
        assertEquals("",m.store.trustedPin("127.0.0.1",""));assertEquals("c".repeat(64),m.store.trustedPin("127.0.0.1",PC));assertEquals("d".repeat(64),m.store.trustedPin("127.0.0.1",OTHER));
        m.store.setToken(OTHER,"b".repeat(64));m.store.forgetComputer(PC);
        assertEquals(List.of(OTHER),m.store.visibleSavedRecords());assertEquals(OTHER,m.store.storedIdentityOrEmpty(usb));assertEquals("b".repeat(64),m.store.token(OTHER));
    }
    @Test public void usbFirstThenWirelessKeepsOneRecordAndForgetsBothEndpoints() {
        Memory m=new Memory();String usb="https://127.0.0.1:49821";m.bind(usb,PC);m.store.rememberComputer(usb);m.store.togglePinned(PC);m.bind(FIRST,PC);m.store.rememberComputer(FIRST);
        assertEquals(List.of(PC),m.store.visibleSavedRecords());assertEquals(FIRST,m.store.endpointFor(PC));assertEquals(usb,m.store.usbEndpoint(PC));
        assertTrue(m.store.isPinned(PC));m.store.togglePinned(PC);assertFalse(m.store.isPinned(PC));
        m.store.forgetComputer(PC);assertTrue(m.store.visibleSavedRecords().isEmpty());assertEquals("",m.store.usbEndpoint(PC));assertEquals("",m.store.lastIdentity());
    }
    @Test public void reusedWirelessAddressKeepsBothComputersAndDeletingTheOldOnePreservesTheNewOne()throws Exception {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.rememberComputer(FIRST);m.store.setToken(PC,"a".repeat(64));m.store.setMobilePreferences(PC,snapshot("First"));
        m.bind(FIRST,OTHER);m.store.rememberComputer(FIRST);m.store.setToken(OTHER,"b".repeat(64));m.store.setMobilePreferences(OTHER,snapshot("Other"));m.store.saveComputerMetadata(OTHER,"Other PC","");
        assertEquals(Set.of(PC,OTHER),new HashSet<>(m.store.visibleSavedRecords()));m.store.forgetComputer(PC);
        assertEquals(List.of(OTHER),m.store.visibleSavedRecords());assertEquals(OTHER,m.store.storedIdentityOrEmpty(FIRST));assertEquals("b".repeat(64),m.store.token(OTHER));assertEquals("Other PC",m.store.computerMetadata(OTHER,ComputerStore.ALIAS_PREFIX));assertEquals("Other",new JSONObject(m.store.mobilePreferences(OTHER)).getJSONArray("shortcutPlans").getJSONObject(0).getString("name"));assertEquals(OTHER,m.store.lastIdentity());
    }
    @Test public void legacySavedIdentityAndPinnedOrderRemainWithTheOriginalComputer() {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.rememberComputer(FIRST);m.values.remove("saved_computer_identities");m.values.put("pinned_urls",Set.of(FIRST));m.values.put("saved_order",FIRST);
        m.bind(FIRST,OTHER);m.store.rememberComputer(FIRST);assertEquals(List.of(PC,OTHER),m.store.visibleSavedRecords());assertTrue(m.store.isPinned(PC));assertFalse(m.store.isPinned(OTHER));
    }
    @Test public void staleEndpointCredentialsAndMetadataStayWithTheirOriginalOwner() {
        Memory m=new Memory();m.bind(FIRST,PC);m.store.rememberComputer(FIRST);m.store.setToken(FIRST,"a".repeat(64));m.values.put("computer_alias_"+EndpointUtils.sha256Hex(FIRST),"Old PC");
        m.bind(FIRST,OTHER);m.bind(FIRST,OTHER);assertEquals("",m.store.token(OTHER));assertEquals("",m.store.computerMetadata(OTHER,ComputerStore.ALIAS_PREFIX));assertEquals("a".repeat(64),m.store.token(PC));assertEquals("Old PC",m.store.computerMetadata(PC,ComputerStore.ALIAS_PREFIX));
    }
    @Test public void aLateConnectionSnapshotCannotRememberTheOtherOwnerOfTheSameEndpoint() {
        Memory m=new Memory();String usb="https://127.0.0.1:49821";m.bind(usb,PC);m.store.rememberComputer(usb);m.bind(usb,OTHER);m.store.rememberComputer(usb);m.store.preferUsb(OTHER,false);
        m.store.rememberComputer(usb,PC);assertEquals(OTHER,m.store.lastIdentity());assertFalse(m.values.get("prefer_usb_"+OTHER).equals(true));
    }
    @Test public void unknownLoopbackCopiesAreNotAssignedToTheNextTrustedComputer()throws Exception {
        Memory m=new Memory();String usb="https://127.0.0.1:49821";m.old(usb,snapshot("Unknown"));m.store.setToken(usb,"a".repeat(64));m.bind(usb,PC);
        m.bind(usb,PC);m.bind(usb,OTHER);assertEquals("",m.store.token(PC));assertEquals("",m.store.mobilePreferences(PC));assertEquals("",m.store.token(OTHER));assertEquals("",m.store.mobilePreferences(OTHER));
    }
    @Test public void removingOnePcDoesNotRemoveTheUsbKeyNowOwnedByAnotherPc() {
        Memory m=new Memory();String usb="https://127.0.0.1:49821",next="https://127.0.0.1:49822";
        m.bind(usb,PC);m.store.rememberComputer(usb);m.store.setUsbAccess(usb,"a".repeat(64));
        m.bind(usb,OTHER);m.store.rememberComputer(usb);m.store.setUsbAccess(usb,"b".repeat(64));m.store.forgetComputer(PC);
        assertEquals("b".repeat(64),m.store.usbAccess(usb));m.bind(next,OTHER);assertEquals("",m.store.usbAccess(usb));
        m.store.setUsbAccess(next,"c".repeat(64));m.store.forgetComputer(OTHER);assertEquals("",m.store.usbAccess(next));
    }
}
