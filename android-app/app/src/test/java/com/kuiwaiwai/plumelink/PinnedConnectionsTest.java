package com.kuiwaiwai.plumelink;

import java.lang.reflect.Proxy;
import java.util.concurrent.atomic.AtomicReference;
import okhttp3.Interceptor;
import okhttp3.OkHttpClient;
import okhttp3.Protocol;
import okhttp3.Request;
import okhttp3.Response;
import org.junit.Test;
import static org.junit.Assert.*;

public class PinnedConnectionsTest {
    private static final String BASE="https://127.0.0.1:49821",HEADER="X-PlumeLink-USB",KEY="a".repeat(64);

    private Request intercept(OkHttpClient client,String url)throws Exception {
        Request input=new Request.Builder().url(url).header(HEADER,"stale").build();
        AtomicReference<Request> sent=new AtomicReference<>();
        Interceptor.Chain chain=(Interceptor.Chain)Proxy.newProxyInstance(Interceptor.Chain.class.getClassLoader(),new Class<?>[]{Interceptor.Chain.class},(proxy,method,args)->{
            if("request".equals(method.getName()))return input;
            if("proceed".equals(method.getName())){
                sent.set((Request)args[0]);
                return new Response.Builder().request(sent.get()).protocol(Protocol.HTTP_1_1).code(200).message("OK").build();
            }
            throw new AssertionError(method.getName());
        });
        assertEquals(1,client.interceptors().size());
        client.interceptors().get(0).intercept(chain);
        return sent.get();
    }

    @Test public void microphoneAndTransferClientsAuthorizeOnlyTheCapturedUsbOrigin()throws Exception {
        PinnedConnections network=new PinnedConnections("127.0.0.1","pin",BASE,KEY);
        for(OkHttpClient client:new OkHttpClient[]{network.audioClient(),network.transferClient().newBuilder().build()}){
            assertEquals(KEY,intercept(client,BASE+"/api/uploads/owned").header(HEADER));
            assertEquals(KEY,intercept(client,"wss://127.0.0.1:49821/microphone?resume=1").header(HEADER));
            for(String other:new String[]{"https://127.0.0.1:49822/api/files","https://192.168.1.2:49821/api/files","http://127.0.0.1:49821/api/files"})
                assertNull(intercept(client,other).header(HEADER));
            assertFalse(client.followRedirects());assertFalse(client.followSslRedirects());
        }
    }

    @Test public void oldNetworkInstancesKeepTheirOriginalAccessForDelayedCleanup()throws Exception {
        PinnedConnections previous=new PinnedConnections("127.0.0.1","pin",BASE,KEY);
        PinnedConnections current=new PinnedConnections("127.0.0.1","pin",BASE,"b".repeat(64));
        assertEquals("b".repeat(64),intercept(current.transferClient(),BASE+"/api/uploads/new").header(HEADER));
        assertEquals(KEY,intercept(previous.transferClient(),BASE+"/api/uploads/previous").header(HEADER));
        assertEquals(KEY,previous.open(BASE+"/api/status","").getRequestProperty(HEADER));
        assertNull(previous.open("https://127.0.0.1:49822/api/status","").getRequestProperty(HEADER));
    }

    @Test public void legacyWirelessAndInvalidAccessNeverAddUsbCredentials()throws Exception {
        for(PinnedConnections network:new PinnedConnections[]{new PinnedConnections("127.0.0.1","pin"),
            new PinnedConnections("127.0.0.1","pin",BASE,"invalid"),
            new PinnedConnections("192.168.1.2","pin","https://192.168.1.2:17840",KEY),
            new PinnedConnections("127.0.0.1","pin","https://user@127.0.0.1:49821",KEY)})
            assertNull(intercept(network.audioClient(),BASE+"/microphone").header(HEADER));
    }
}
