package com.kuiwaiwai.plumelink;

import android.annotation.SuppressLint;
import android.content.Context;
import android.net.nsd.NsdManager;
import android.net.nsd.NsdServiceInfo;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;

import java.net.Inet4Address;
import java.net.Inet6Address;
import java.net.InetAddress;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

final class LanDiscovery {
    private final Context context;
    private final Runnable onUpdate;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final LinkedHashMap<String, String> computers = new LinkedHashMap<>();
    private final LinkedHashMap<String, String> identities = new LinkedHashMap<>();
    private final Set<String> lostServices = new HashSet<>();
    private final ArrayDeque<NsdServiceInfo> pendingResolves = new ArrayDeque<>();
    private final List<Object> activeServiceCallbacks = new ArrayList<>();

    private NsdManager.DiscoveryListener listener;
    private WifiManager.MulticastLock multicastLock;
    private boolean resolvingService;
    private String resolvingServiceName = "";
    private Runnable timeout;
    private int generation = 0;
    private boolean finished;
    private String error = "";

    LanDiscovery(Context context, Runnable onUpdate) {
        this.context = context.getApplicationContext();
        this.onUpdate = onUpdate;
    }

    private String ui(int id) { return AppLanguage.text(context, id); }

    void resetState() {
        computers.clear();
        identities.clear();
        lostServices.clear();
        finished = false;
        error = "";
    }

    Map<String, String> computers() { return computers; }
    String identityFor(String url) { return identities.getOrDefault(url, ""); }
    boolean isFinished() { return finished; }
    String error() { return error; }

    @SuppressLint("MissingPermission")
    void start() {
        if (listener != null) return;
        finished = false;
        error = "";
        onUpdate.run();
        final int currentGen = generation;
        WifiManager wifi = (WifiManager) context.getSystemService(Context.WIFI_SERVICE);
        if (wifi != null) {
            multicastLock = wifi.createMulticastLock("PlumeLinkDiscovery");
            multicastLock.setReferenceCounted(false);
            multicastLock.acquire();
        }
        NsdManager manager = (NsdManager) context.getSystemService(Context.NSD_SERVICE);
        listener = new NsdManager.DiscoveryListener() {
            @Override public void onDiscoveryStarted(String type) {}
            @Override public void onDiscoveryStopped(String type) {}
            @Override public void onStartDiscoveryFailed(String type, int errorCode) {
                handler.post(() -> {
                    if (currentGen != generation) return;
                    stop();
                    finished = true;
                    error = ui(R.string.ui_search_failed) + errorCode + ui(R.string.ui_search_again_or_connect_by_address);
                    onUpdate.run();
                });
            }
            @Override public void onStopDiscoveryFailed(String type, int errorCode) {}
            @Override public void onServiceLost(NsdServiceInfo service) {
                handler.post(() -> {
                    if (currentGen != generation) return;
                    String serviceName = service.getServiceName();
                    if (serviceName != null && !serviceName.isEmpty()) {
                        lostServices.add(serviceName);
                        pendingResolves.removeIf(item -> serviceName.equals(item.getServiceName()));
                    }
                    computers.entrySet().removeIf(entry -> {
                        if (!entry.getValue().equals(service.getServiceName())) return false;
                        identities.remove(entry.getKey());
                        return true;
                    });
                    onUpdate.run();
                });
            }
            @Override public void onServiceFound(NsdServiceInfo service) {
                if (!service.getServiceType().startsWith("_plumelink._tcp")) return;
                handler.post(() -> {
                    if (currentGen != generation) return;
                    String serviceName = service.getServiceName();
                    if (serviceName == null || serviceName.isEmpty()) return;
                    lostServices.remove(serviceName);
                    if (serviceName.equals(resolvingServiceName)) return;
                    for (NsdServiceInfo queued : pendingResolves) {
                        if (serviceName.equals(queued.getServiceName())) return;
                    }
                    pendingResolves.add(service);
                    resolveNext(manager, currentGen);
                });
            }
        };
        try {
            manager.discoverServices("_plumelink._tcp.", NsdManager.PROTOCOL_DNS_SD, listener);
            timeout = () -> {
                if (currentGen != generation) return;
                // End the initial search indicator while continuing to receive additions and removals.
                timeout = null;
                finished = true;
                onUpdate.run();
            };
            handler.postDelayed(timeout, 8000);
        } catch (Exception ignored) {
            stop();
            finished = true;
            error = ui(R.string.ui_search_failed_try_again_scan_the_qr_code_in_the_app_or);
            onUpdate.run();
        }
    }

    void retry() {
        stop();
        computers.clear();
        identities.clear();
        start();
    }

    @SuppressLint("NewApi")
    void stop() {
        generation++;
        pendingResolves.clear();
        lostServices.clear();
        resolvingService = false;
        resolvingServiceName = "";
        if (timeout != null) handler.removeCallbacks(timeout);
        timeout = null;
        NsdManager manager = (NsdManager) context.getSystemService(Context.NSD_SERVICE);
        if (Build.VERSION.SDK_INT >= 34 && manager != null && !activeServiceCallbacks.isEmpty()) {
            for (Object item : new ArrayList<>(activeServiceCallbacks)) {
                try { manager.unregisterServiceInfoCallback((NsdManager.ServiceInfoCallback) item); } catch (Exception ignored) {}
            }
        }
        activeServiceCallbacks.clear();
        if (listener != null) {
            try { if (manager != null) manager.stopServiceDiscovery(listener); } catch (Exception ignored) {}
            listener = null;
        }
        if (multicastLock != null && multicastLock.isHeld()) multicastLock.release();
        multicastLock = null;
    }

    private void recordResolved(String serviceName, InetAddress hostAddress, int port, Map<String, byte[]> attrs) {
        if (!(hostAddress instanceof Inet4Address) && !(hostAddress instanceof Inet6Address)) return;
        if (hostAddress instanceof Inet6Address && hostAddress.isLinkLocalAddress()) return;
        String rawHost = hostAddress.getHostAddress();
        if (rawHost == null) return;
        int zone = rawHost.indexOf('%');
        if (zone > 0) rawHost = rawHost.substring(0, zone);
        if (!EndpointUtils.isPrivateHost(rawHost) || port <= 0 || port > 65535) return;
        String urlHost = hostAddress instanceof Inet6Address ? "[" + rawHost + "]" : rawHost;
        String url = "https://" + urlHost + ":" + port;
        byte[] idBytes = attrs == null ? null : attrs.get("id");
        String stableId = idBytes == null ? "" : new String(idBytes, StandardCharsets.UTF_8);
        if (stableId.matches("^[a-f0-9]{24}$")) {
            identities.entrySet().removeIf(entry -> {
                if (!stableId.equals(entry.getValue())) return false;
                computers.remove(entry.getKey());
                return true;
            });
            identities.put(url, stableId);
        }
        error = "";
        computers.put(url, serviceName);
        onUpdate.run();
    }

    @SuppressLint("NewApi")
    private void resolveWithCallback(NsdManager manager, NsdServiceInfo service, int currentGen) {
        final Runnable[] timeoutRef = new Runnable[1];
        final boolean[] completed = new boolean[1];
        NsdManager.ServiceInfoCallback callback = new NsdManager.ServiceInfoCallback() {
            private void finish(boolean failed) {
                if (completed[0]) return;
                completed[0] = true;
                if (timeoutRef[0] != null) handler.removeCallbacks(timeoutRef[0]);
                activeServiceCallbacks.remove(this);
                try { manager.unregisterServiceInfoCallback(this); } catch (Exception ignored) {}
                if (currentGen != generation) return;
                resolvingService = false;
                resolvingServiceName = "";
                if (failed && computers.isEmpty()) {
                    error = ui(R.string.ui_pc_found_but_its_address_could_not_be_resolved_search_a);
                    onUpdate.run();
                }
                resolveNext(manager, currentGen);
            }
            @Override public void onServiceInfoCallbackRegistrationFailed(int errorCode) { finish(true); }
            @Override public void onServiceUpdated(NsdServiceInfo info) {
                InetAddress chosen = null;
                for (InetAddress address : info.getHostAddresses()) {
                    if (address instanceof Inet4Address && EndpointUtils.isPrivateHost(address.getHostAddress())) {
                        chosen = address;
                        break;
                    }
                }
                if (chosen == null) {
                    for (InetAddress address : info.getHostAddresses()) {
                        if (address instanceof Inet6Address && !address.isLinkLocalAddress() && EndpointUtils.isPrivateHost(address.getHostAddress())) {
                            chosen = address;
                            break;
                        }
                    }
                }
                if (chosen == null && info.getHost() instanceof Inet4Address) chosen = info.getHost();
                if (chosen == null || info.getPort() <= 0 || info.getPort() > 65535) return;
                if (currentGen == generation && !lostServices.contains(info.getServiceName())) {
                    recordResolved(info.getServiceName(), chosen, info.getPort(), info.getAttributes());
                }
                finish(false);
            }
            @Override public void onServiceLost() { finish(true); }
            @Override public void onServiceInfoCallbackUnregistered() {}
        };
        activeServiceCallbacks.add(callback);
        timeoutRef[0] = () -> {
            if (!completed[0]) {
                completed[0] = true;
                activeServiceCallbacks.remove(callback);
                try { manager.unregisterServiceInfoCallback(callback); } catch (Exception ignored) {}
                if (currentGen != generation) return;
                resolvingService = false;
                resolvingServiceName = "";
                resolveNext(manager, currentGen);
            }
        };
        handler.postDelayed(timeoutRef[0], 2500);
        try {
            manager.registerServiceInfoCallback(service, handler::post, callback);
        } catch (Exception ignored) {
            if (timeoutRef[0] != null) handler.removeCallbacks(timeoutRef[0]);
            activeServiceCallbacks.remove(callback);
            resolvingService = false;
            resolvingServiceName = "";
            resolveNext(manager, currentGen);
        }
    }

    @SuppressWarnings("deprecation")
    private void resolveNext(NsdManager manager, int currentGen) {
        if (currentGen != generation || resolvingService || listener == null || pendingResolves.isEmpty()) return;
        NsdServiceInfo next = pendingResolves.poll();
        if (next == null) return;
        resolvingService = true;
        resolvingServiceName = next.getServiceName() == null ? "" : next.getServiceName();
        if (Build.VERSION.SDK_INT >= 34) {
            resolveWithCallback(manager, next, currentGen);
            return;
        }
        try {
            manager.resolveService(next, new NsdManager.ResolveListener() {
                @Override public void onResolveFailed(NsdServiceInfo info, int errorCode) {
                    handler.post(() -> {
                        if (currentGen != generation) return;
                        resolvingService = false;
                        resolvingServiceName = "";
                        if (computers.isEmpty()) {
                            error = ui(R.string.ui_pc_found_but_its_address_could_not_be_resolved_search_a);
                            onUpdate.run();
                        }
                        resolveNext(manager, currentGen);
                    });
                }
                @Override public void onServiceResolved(NsdServiceInfo info) {
                    handler.post(() -> {
                        if (currentGen != generation) return;
                        resolvingService = false;
                        resolvingServiceName = "";
                        if (!lostServices.contains(info.getServiceName())) {
                            recordResolved(info.getServiceName(), info.getHost(), info.getPort(), info.getAttributes());
                        }
                        resolveNext(manager, currentGen);
                    });
                }
            });
        } catch (Exception ignored) {
            resolvingService = false;
            resolvingServiceName = "";
            resolveNext(manager, currentGen);
        }
    }
}
