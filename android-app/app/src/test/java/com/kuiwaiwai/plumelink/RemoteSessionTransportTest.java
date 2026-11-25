package com.kuiwaiwai.plumelink;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.LinkedHashMap;
import java.util.Map;
import org.json.JSONObject;
import org.junit.Test;
import static org.junit.Assert.*;

public class RemoteSessionTransportTest {
    @Test public void lateActivityCallbacksCannotRetireTheNewForegroundOwner() throws Exception {
        RemoteSessionService service = new RemoteSessionService();
        Object old = new Object(), current = new Object();
        assertTrue(service.attachControlHost(old, true));
        assertTrue(service.attachControlHost(current, true));
        service.background(old); assertFalse(service.detachControlHost(old));
        assertTrue(service.isControlHost(current)); assertFalse(service.attachControlHost(old, false));
        assertTrue(service.detachControlHost(current)); assertFalse(service.isControlHost(current));
    }
    @Test public void backgroundTerminalResultsCannotCrossOwnersTransportLeasesOrCredentials() throws Exception {
        RemoteSessionService service = new RemoteSessionService();
        set(service,"selectedOwner","pc-one"); set(service,"selectedTransport","usb-old");
        set(service,"controlToken","a".repeat(64)); set(service,"controlTerminalCode",4409);
        Method emit = RemoteSessionService.class.getDeclaredMethod("emitKeepAlive"); emit.setAccessible(true); emit.invoke(service);
        assertEquals(4409,new JSONObject(service.connectionKeepAliveState("a".repeat(64),"pc-one","usb-old")).getInt("terminalCode"));
        assertEquals("{}",service.connectionKeepAliveState("b".repeat(64),"pc-one","usb-old"));
        assertEquals("{}",service.connectionKeepAliveState("a".repeat(64),"pc-two","usb-old"));
        assertEquals("{}",service.connectionKeepAliveState("a".repeat(64),"pc-one","lan-or-new-usb"));
        service.setControlCredentials("b".repeat(64));
        assertEquals(0,new JSONObject(service.connectionKeepAliveState("b".repeat(64),"pc-one","usb-old")).getInt("terminalCode"));
    }
    private void set(RemoteSessionService service,String name,Object value)throws Exception {
        Field field=RemoteSessionService.class.getDeclaredField(name);field.setAccessible(true);field.set(service,value);
    }
    private void invoke(RemoteSessionService service,String name,Class<?> argument,Object value)throws Exception {
        Method method=RemoteSessionService.class.getDeclaredMethod(name,argument);method.setAccessible(true);method.invoke(service,value);
    }
    private void close(RemoteSessionService service)throws Exception {
        Field field=RemoteSessionService.class.getDeclaredField("recoveryThreads");field.setAccessible(true);for(Object worker:((Map<?,?>)field.get(service)).values())((Thread)worker).interrupt();
    }

    @Test public void retainedTaskSnapshotsKeepTheirOriginalTransportAfterAccessRotates()throws Exception {
        RemoteSessionService service=new RemoteSessionService();
        try {
            Map<String,JSONObject> events=new LinkedHashMap<>();
            set(service,"listener",(RemoteSessionService.Listener)events::put);
            set(service,"selectedOwner","same-computer");set(service,"selectedTransport","new-transport");
            set(service,"baseOwner","same-computer");set(service,"baseTransport","old-microphone");
            set(service,"transferOwner","same-computer");set(service,"transferTransport","old-transfer");
            invoke(service,"emitMic",boolean.class,true);invoke(service,"emitTransfer",boolean.class,true);
            assertEquals("old-microphone",events.get("plumelink-microphone").getString("transport"));
            assertEquals("old-transfer",events.get("plumelink-native-transfer").getString("transport"));
            for(JSONObject event:events.values())assertEquals("same-computer",event.getString("owner"));
        } finally {close(service);}
    }

    @Test public void recoverySnapshotUsesTheCurrentTransportWithoutRetaggingTasks()throws Exception {
        RemoteSessionService service=new RemoteSessionService();
        try {
            Map<String,JSONObject> events=new LinkedHashMap<>();
            set(service,"listener",(RemoteSessionService.Listener)events::put);
            set(service,"selectedOwner","same-computer");set(service,"selectedTransport","new-transport");
            set(service,"transferOwner","same-computer");set(service,"transferTransport","old-transfer");
            Method emit=RemoteSessionService.class.getDeclaredMethod("emitRecovery");emit.setAccessible(true);emit.invoke(service);
            JSONObject event=events.get("plumelink-download-recovery");
            assertEquals("same-computer",event.getString("owner"));assertEquals("new-transport",event.getString("transport"));
            assertEquals(0,event.getJSONArray("files").length());
            invoke(service,"emitTransfer",boolean.class,true);
            assertEquals("old-transfer",events.get("plumelink-native-transfer").getString("transport"));
        } finally {close(service);}
    }
}
