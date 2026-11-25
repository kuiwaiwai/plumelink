import { t } from "../../shared/i18n";
import { androidBridge } from "./native/bridge";
import { computed, ref } from "vue";
import type { Command } from "../../shared/protocol";
import type { Motion } from "./qft-transport";
import { storage } from "./storage";
import { loadQftKernel, QftTransport, supportsQft } from "./qft-transport";
import { thumbnail } from "../../shared/thumbnail";

export type ConnectionQuality = "unknown" | "good" | "fair" | "poor";

export type RemoteWindow = {
  hwnd: number;
  title: string;
  app: string;
  icon: string | null;
  minimized: boolean;
};

function errorText(error: unknown, fallback: string) {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  if (error && typeof error === "object") {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.trim()) return message;
  }
  return fallback;
}

export function classifyConnectionQuality(
  latency: number | null,
  jitter: number | null,
): ConnectionQuality {
  if (latency === null || !Number.isFinite(latency) || latency < 0) return "unknown";
  const variation = jitter === null || !Number.isFinite(jitter) || jitter < 0 ? 0 : jitter;
  if (latency <= 120 && variation <= 30) return "good";
  if (latency <= 300 && variation <= 100) return "fair";
  return "poor";
}

export function useConnection() {
  const usb = typeof location !== "undefined" && location.hostname === "127.0.0.1";
  const status = ref<"unpaired" | "connecting" | "connected" | "offline">(
    "unpaired",
  );
  const networkOffline = ref(!usb && typeof navigator !== "undefined" && navigator.onLine === false);
  const message = ref(t('输入电脑上的配对码，开始遥控'));
  const latency = ref<number | null>(null);
  const jitter = ref<number | null>(null);
  const quality = computed(() => classifyConnectionQuality(latency.value, jitter.value));
  const qualityLabel = computed(() => ({
    unknown: t('等待网络质量'),
    good: t('网络稳定'),
    fair: t('网络一般'),
    poor: t('网络较差'),
  })[quality.value]);
  const features = ref<string[]>([]);
  const permissions = ref({ input: true, text: true, files: true, power: false, microphone: false });
  const deviceName = ref("");
  const controlMode = ref<"free" | "primary">("free");
  const primary = ref(true);
  const connectedCount = ref(1);
  const primaryName = ref("");
  const controlRequestPending = ref(false);
  let requestAcknowledgedAt = 0;
  let requestGeneration = 0;
  let requestExpiry: ReturnType<typeof setTimeout> | undefined;
  function clearControlRequest() {
    requestGeneration++;
    clearTimeout(requestExpiry);
    controlRequestPending.value = false;
  }
  function waitForControl() {
    controlRequestPending.value = true;
    requestAcknowledgedAt = performance.now();
    clearTimeout(requestExpiry);
    requestExpiry = setTimeout(clearControlRequest, 60_000);
  }
  const adminGranted = ref(false);
  const recentController = ref("");
  const usbExpired = ref(false);
  const phase = ref<"initial" | "retrying" | "failed" | "paused" | "replaced">("initial");
  const connectionLabel = computed(() => {
    if (usbExpired.value) return t("USB 连接已失效");
    if (status.value === "connected") return latency.value === null ? t('已连接') : t('已连接 · {0} ms', latency.value);
    if (status.value === "unpaired") return t('点击配对');
    if (networkOffline.value && status.value === "offline") return t("网络已断开，等待网络恢复…");
    if (phase.value === "replaced") return t('连接已被接管，点击重连');
    if (phase.value === "paused") return t('连接已暂停，点击重连');
    if (phase.value === "failed") return t('连接失败，点击重试');
    if (phase.value === "retrying") return t('连接已断开，正在重连…');
    return t('正在连接…');
  });
  const connectionBadgeLabel = computed(() => {
    if (usbExpired.value) return t("USB 连接已失效");
    if (status.value === "connected") return t("已连接");
    if (status.value === "unpaired") return t("连接电脑");
    if (networkOffline.value && status.value === "offline") return t("无网络");
    if (phase.value === "retrying") return t("正在重连…");
    if (phase.value === "failed") return t("连接失败");
    if (phase.value === "paused" || phase.value === "replaced") return t("未连接");
    return t("连接中");
  });
  let token = "";
  let deviceId = "";
  const nativeCredentials = androidBridge();
  let usbAccess = "";
  try { const key = nativeCredentials?.getUsbAccessKey?.(); if (usb && typeof key === "string" && /^[a-f0-9]{64}$/.test(key)) usbAccess = key; } catch {}
  const usbHeaders = (): Record<string,string> => usbAccess ? { "X-PlumeLink-USB": usbAccess } : {};
  function expireUsb() {
    usbExpired.value = true; suspend(); phase.value = "failed";
    message.value = t('USB 连接已失效，请在电脑「偏好设置 → USB 有线连接」中重新连接手机');
  }
  try {
    token = nativeCredentials?.getStoredToken ? nativeCredentials.getStoredToken() : storage.getItem("plumelink-token") || "";
    deviceId = nativeCredentials?.getStoredDeviceId ? nativeCredentials.getStoredDeviceId() : storage.getItem("plumelink-device-id") || "";
  } catch {}
  const fileAccessPending = computed(() => (
    status.value !== "connected" && status.value !== "unpaired" && phase.value !== "replaced" && !!token
  ));
  if (!deviceId) {
    deviceId = typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : Array.from(crypto.getRandomValues(new Uint8Array(16)), value => value.toString(16).padStart(2, "0")).join("");
    try {
      storage.setItem("plumelink-device-id", deviceId);
      nativeCredentials?.setStoredDeviceId?.(deviceId);
    } catch {}
  }
  let socket: WebSocket | undefined;
  async function syncDeviceName(ws: WebSocket) {
    try {
      const name = nativeCredentials?.getDeviceName?.();
      if (typeof name !== "string" || !name.trim()) return;
      const model = nativeCredentials?.getDeviceModel?.() || "";
      const result = await api("/api/device/name", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name, model }),
      });
      if (!result.ok || socket !== ws || ws.readyState !== WebSocket.OPEN) return;
      const body = await result.json();
      if (socket === ws && typeof body.deviceName === "string" && body.deviceName.length <= 80
          && !/[\u0000-\u001f\u007f]/.test(body.deviceName)) deviceName.value = body.deviceName;
    } catch {}
  }
  let retry: ReturnType<typeof setTimeout> | undefined;
  let heartbeat: ReturnType<typeof setInterval> | undefined;
  let attempt = 0;
  let lastPong = 0;
  let pingAt = 0;
  let suspended = false;
  let backgroundAllowed = false;
  const mayRun = () => !document.hidden || backgroundAllowed;
  function setBackgroundAllowed(enabled: boolean) { backgroundAllowed = enabled; }
  let pairingController: AbortController | undefined;
  let credentialGeneration = 0;
  let samples: number[] = [];
  let responseMode = "balanced";
  const qft2 = ref(false);
  let qft: QftTransport | undefined;
  let qftSwitchTimer: ReturnType<typeof setTimeout> | undefined;
  let networkListenersAttached = false;
  const onNetworkOnline = () => {
    networkOffline.value = false;
    if (!token || suspended || !mayRun()) return;
    attempt = 0;
    if (!socket || socket.readyState >= WebSocket.CLOSING) connect();
  };
  const onNetworkOffline = () => {
    if (suspended || usb) return;
    networkOffline.value = true;
    clearTimeout(retry);
    if (socket && socket.readyState < WebSocket.CLOSING) return;
    resetSession();
    status.value = token ? "offline" : "unpaired";
    message.value = t('网络已断开，等待网络恢复…');
    scheduleReconnect();
  };
  function setNetworkListeners(attach: boolean) {
    if (typeof window === "undefined" || typeof window.addEventListener !== "function") return;
    if (attach && !networkListenersAttached) {
      window.addEventListener("online", onNetworkOnline);
      window.addEventListener("offline", onNetworkOffline);
      networkListenersAttached = true;
    } else if (!attach && networkListenersAttached) {
      window.removeEventListener("online", onNetworkOnline);
      window.removeEventListener("offline", onNetworkOffline);
      networkListenersAttached = false;
    }
  }
  const emptyStats = () => ({ intervalMs: 16, srttMs: null as number | null, queueMs: 0, sent: 0, merged: 0, expired: 0, pressured: 0 });
  const transportStats = ref(emptyStats());
  function clearMotion() { qft?.clearUnsent(); }
  function resetTransport() {
    clearTimeout(qftSwitchTimer);
    qft?.dispose(); qft = undefined; qft2.value = false;
    transportStats.value = emptyStats();
  }
  function setResponseMode(mode: string) {
    if (mode !== "balanced" && mode !== "responsive") return;
    const changed = responseMode !== mode;
    responseMode = mode;
    if (changed) qft?.setIntervalUs(mode === "responsive" ? 12_000 : 16_000);
  }
  const resetQuality = () => {
    latency.value = null;
    jitter.value = null;
    samples = [];
    pingAt = 0;
  };
  const recordLatency = (sample: number) => {
    if (!Number.isFinite(sample) || sample < 0 || sample > 60_000) return;
    sample = Math.round(sample);
    samples = [...samples.slice(-7), sample];
    latency.value = sample;
    if (samples.length > 1) {
      jitter.value = Math.round(
        samples.slice(1).reduce((sum, value, index) => sum + Math.abs(value - samples[index]), 0) /
          (samples.length - 1),
      );
    }
  };
  const saveToken = (value: string) => {
    token = value;
    try {
      value
        ? storage.setItem("plumelink-token", value)
        : storage.removeItem("plumelink-token");
      nativeCredentials?.setStoredToken?.(value);
    } catch {}
  };
  function applyPresence(value: unknown, pingSentAt: number) {
    if (!value || typeof value !== "object") return;
    const presence = value as Record<string, unknown>;
    const hasControlMode = presence.controlMode === "free" || presence.controlMode === "primary";
    if (!hasControlMode) {
      controlMode.value = "free";
      primary.value = false;
    }
    if (hasControlMode) {
      controlMode.value = presence.controlMode === "primary" ? "primary" : "free";
      // Primary mode requires an explicit grant in the presence frame.
      if (presence.controlMode === "primary" && typeof presence.primary !== "boolean")
        primary.value = false;
      if (presence.controlMode === "free") primary.value = false;
    }
    if (presence.controlMode === "primary" && typeof presence.primary === "boolean")
      primary.value = presence.primary;
    if (controlMode.value === "free" || primary.value || !permissions.value.input) {
      if (controlRequestPending.value) clearControlRequest();
    } else if (presence.controlRequestPending === true && !controlRequestPending.value) {
      waitForControl();
    } else if (presence.controlRequestPending === false && controlRequestPending.value && pingSentAt > requestAcknowledgedAt) {
      clearControlRequest();
    }
    if (typeof presence.connectedCount === "number" && Number.isFinite(presence.connectedCount)) connectedCount.value = Math.max(1, Math.min(32, Math.floor(presence.connectedCount)));
    if (typeof presence.primaryName === "string" && presence.primaryName.length <= 80 && !/[\u0000-\u001f\u007f]/.test(presence.primaryName)) primaryName.value = presence.primaryName;
    else if (presence.primaryName === null) primaryName.value = "";
    if (typeof presence.adminGranted === "boolean") adminGranted.value = presence.adminGranted;
    if (typeof presence.recentController === "string" && presence.recentController.length <= 80 && !/[\u0000-\u001f\u007f]/.test(presence.recentController)) recentController.value = presence.recentController;
    else if (presence.recentController === null) recentController.value = "";
  }
  function scheduleReconnect() {
    clearTimeout(retry);
    if (!token || suspended || !mayRun()) return;
    const base = networkOffline.value ? 5000 : Math.min(5000, 500 * 2 ** Math.min(attempt++, 6));
    retry = setTimeout(connect, base + Math.floor(Math.random() * 200));
  }
  function resetSession() {
    clearControlRequest();
    clearInterval(heartbeat);
    resetQuality();
    resetTransport();
    features.value = [];
    permissions.value = { input: false, text: false, files: false, power: false, microphone: false };
    controlMode.value = "free";
    primary.value = false;
    connectedCount.value = 0;
    primaryName.value = "";
    adminGranted.value = false;
    recentController.value = "";
  }
  function connect() {
    clearTimeout(retry);
    if (!token || !mayRun() || suspended) return;
    if (nativeCredentials?.isControlForegroundReady?.() === false) return;
    if (socket && socket.readyState < WebSocket.CLOSING) return;
    if (phase.value === "failed") phase.value = "retrying";
    clearInterval(heartbeat);
    status.value = "connecting";
    message.value = t('正在连接电脑…');
    let ws: WebSocket;
    try {
      ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws${usbAccess ? `?usb=${usbAccess}` : ""}`);
    } catch {
      resetSession();
      status.value = "offline";
      phase.value = "failed";
      message.value = t('连接失败，请检查电脑服务、Wi-Fi 和防火墙');
      scheduleReconnect();
      return;
    }
    socket = ws;
    let receivedReady = false;
    ws.binaryType = "arraybuffer";
    let connectionHeartbeat: ReturnType<typeof setInterval> | undefined;
    const openTimeout = setTimeout(() => {
      if (status.value !== "connected") ws.close();
    }, 5000);
    ws.onopen = () => {
      if (socket !== ws || suspended) { ws.close(); return; }
      try { ws.send(JSON.stringify({ kind: "auth", token })); } catch { ws.close(); }
    };
    ws.onmessage = (event) => {
      if (socket !== ws) return;
      if (event.data instanceof ArrayBuffer) {
        try {
          if (qft) { qft.receive(event.data); return; }
          throw new Error("Unexpected binary frame");
        } catch { ws.close(); }
        return;
      }
      let data;
      try {
        data = JSON.parse(event.data);
        if (!data || typeof data !== "object" || Array.isArray(data)) throw new Error("Invalid frame");
      } catch {
        ws.close();
        return;
      }
      if (data.kind === "qftFailure") {
        qft?.receiveFailure(data);
        return;
      }
      if (data.kind === "ready") {
        if (receivedReady) { ws.close(); return; }
        receivedReady = true;
        clearTimeout(openTimeout);
        status.value = "connected";
        networkOffline.value = false;
        phase.value = "initial";
        message.value = t('已连接，可以遥控');
        attempt = 0;
        lastPong = Date.now();
        resetQuality();
        resetTransport();
        controlMode.value = "free";
        primary.value = false;
        connectedCount.value = 1;
        primaryName.value = "";
        adminGranted.value = false;
        recentController.value = "";
        {
          const nextPermissions = data.permissions && typeof data.permissions === "object"
            ? data.permissions as Record<string, unknown>
            : {};
          permissions.value = {
            input: nextPermissions.input === true,
            text: nextPermissions.text === true,
            files: nextPermissions.files === true,
            power: nextPermissions.power === true,
            microphone: nextPermissions.microphone === true,
          };
        }
        features.value = Array.isArray(data.features) ? ["media", "custom-keys"].filter(feature => data.features.includes(feature)) : [];
        deviceName.value = typeof data.deviceName === "string" && data.deviceName.length <= 80 && !/[\u0000-\u001f\u007f]/.test(data.deviceName) ? data.deviceName : "";
        controlMode.value = data.controlMode === "primary" ? "primary" : "free";
        primary.value = controlMode.value === "primary" && data.primary === true;
        applyPresence(data.presence, 0);
        if (!supportsQft(data.qft)) {
          suspended = true; message.value = t('电脑端版本不匹配，请同时更新手机和电脑端'); ws.close(); return;
        }
        const capability = data.qft;
        void syncDeviceName(ws);
        if (permissions.value.input || permissions.value.text || permissions.value.power) {
          status.value = "connecting";
          let loading = true;
          const failed = () => {
            if (!loading) return;
            loading = false; clearTimeout(loadTimeout);
            if (socket === ws && !suspended && ws.readyState === WebSocket.OPEN) {
              suspended = true; message.value = t('传输组件未能启动，请更新 Android System WebView 后重试'); ws.close();
            }
          };
          const loadTimeout = setTimeout(failed, 3500); qftSwitchTimer = loadTimeout;
          void loadQftKernel().then(kernel => {
            if (!loading) return;
            loading = false; clearTimeout(loadTimeout);
            if (socket !== ws || suspended || ws.readyState !== WebSocket.OPEN) return;
            qft = new QftTransport(kernel, ws, capability, responseMode === "responsive" ? 12_000 : 16_000,
              () => { qft2.value = true; status.value = "connected"; }, error => { message.value = error; });
            qft.start();
          }).catch(failed);
        }
        connectionHeartbeat = setInterval(() => {
          transportStats.value = qft?.snapshot() ?? emptyStats();
          if (Date.now() - lastPong > 4000) {
            ws.close();
            return;
          }
          if (ws.readyState === WebSocket.OPEN && pingAt === 0) {
            pingAt = performance.now();
            try { ws.send(JSON.stringify({ kind: "ping" })); } catch { ws.close(); }
          }
        }, 1000);
        heartbeat = connectionHeartbeat;
      }
      if (data.kind === "pong") {
        lastPong = Date.now();
        const pingSentAt = pingAt;
        if (pingAt) {
          recordLatency(Math.max(1, Math.round(performance.now() - pingAt)));
          pingAt = 0;
        }
        applyPresence(data.presence, pingSentAt);
      }
    };
    ws.onclose = (event) => {
      clearTimeout(openTimeout);
      clearInterval(connectionHeartbeat);
      if (socket !== ws) return;
      resetSession();
      socket = undefined;
      if (event.code === 4409 && usb) { expireUsb(); return; }
      if (event.code === 4003) {
        saveToken("");
        status.value = "unpaired";
        message.value = t('连接已过期，请输入电脑上的配对码');
        return;
      }
      if (event.code === 4001) {
        suspended = true;
        phase.value = "replaced";
        status.value = "offline";
        message.value = t('另一页面已接管连接，点击重新连接可接管回来');
        return;
      }
      const customReason = event.code === 4000 && event.reason && event.reason !== "PlumeLink connection closed" ? event.reason : "";
      if (status.value === "connecting" && phase.value === "initial" && !suspended) phase.value = "failed";
      status.value = token ? "offline" : "unpaired";
      if (!suspended && mayRun() && phase.value !== "failed") phase.value = "retrying";
      if (!suspended) message.value = customReason || (networkOffline.value ? t('网络已断开，等待网络恢复…') : phase.value === "failed" ? usb ? t('USB 连接失败，请检查数据线、手机授权和电脑服务') : t('连接失败，请检查电脑服务、Wi-Fi 和防火墙') : phase.value === "retrying" ? t('连接已断开，正在自动重连电脑…') : t('连接已断开，旧操作已清空'));
      scheduleReconnect();
    };
    ws.onerror = () => {
      if (socket !== ws) return;
      if (!suspended && status.value === "connecting" && phase.value === "initial") {
        phase.value = "failed";
        message.value = t('连接失败，请检查电脑服务、Wi-Fi 和防火墙');
      }
    };
  }
  let pairPending = false;
  async function pair(code: string, name = t('移动设备')) {
    if (usbExpired.value) { message.value=t('USB 连接已失效，请在电脑「偏好设置 → USB 有线连接」中重新连接手机'); return; }
    if (pairPending || !/^[0-9]{6}$/.test(code)) return;
    pairPending = true;
    const generation = credentialGeneration;
    const controller = new AbortController();
    pairingController = controller;
    // First contact can pause TLS while Android shows the certificate fingerprint.
    function checkPairTimeout() {
      if (nativeCredentials?.isAwaitingCertificateTrust?.()) timeout = setTimeout(checkPairTimeout, 6000);
      else controller.abort();
    }
    let timeout = setTimeout(checkPairTimeout, 6000);
    message.value = t('正在配对…');
    try {
      const res = await fetch("/api/pair", {
        method: "POST",
        headers: { "Content-Type": "application/json", ...usbHeaders() },
        body: JSON.stringify({ code, deviceId, deviceName: name }),
        signal: controller.signal,
      });
      if (generation === credentialGeneration && usb && res.status === 410) { expireUsb(); return; }
      const result = await res.json();
      if (generation !== credentialGeneration || controller.signal.aborted) return;
      if (!res.ok) throw new Error(result.error || t('配对失败'));
      if (typeof result.token !== "string" || !/^[a-f0-9]{64}$/i.test(result.token)) throw new Error(t('电脑返回了无效凭证，请重试'));
      suspend();
      saveToken(result.token);
      if (result.deviceId) {
        deviceId = result.deviceId;
        try {
          storage.setItem("plumelink-device-id", deviceId);
          nativeCredentials?.setStoredDeviceId?.(deviceId);
        } catch {}
      }
      suspended = false;
      phase.value = "initial";
      connect();
    } catch (e) {
      if (generation !== credentialGeneration) return;
      message.value = e instanceof TypeError || (e instanceof Error && ["TimeoutError", "AbortError"].includes(e.name)) ? t('网络连接失败，请检查电脑服务和 Wi-Fi 后重试') : errorText(e, t('配对失败，请重试'));
    } finally {
      clearTimeout(timeout);
      if (pairingController === controller) pairingController = undefined;
      pairPending = false;
    }
  }
  function usable() { return status.value === "connected" && socket?.readyState === WebSocket.OPEN && !document.hidden; }
  function sendMotion(motion: Motion) {
    if (usable() && qft && (motion.dx || motion.dy || motion.scroll)) qft.motion(motion);
  }
  function send(command: Command): Promise<void> {
    if (!usable() || !qft) return Promise.reject(new Error(t('尚未连接，操作未发送')));
    if (command.type === "media" && !features.value.includes("media"))
      return Promise.reject(new Error(t("请更新电脑端以使用系统媒体控制")));
    return qft.send(command);
  }
  function suspend() {
    credentialGeneration++;
    pairingController?.abort();
    suspended = true;
    phase.value = usbExpired.value ? "failed" : "paused";
    setNetworkListeners(false);
    clearTimeout(retry);
    qft?.cancel();
    resetSession();
    const previous = socket;
    socket = undefined;
    previous?.close();
    status.value = token ? "offline" : "unpaired";
  }
  function releaseInput() {
    suspend();
  }
  function resume() {
    if (usbExpired.value) return;
    if (nativeCredentials?.isControlForegroundReady?.() === false) return;
    try {
      const state = JSON.parse(nativeCredentials?.getConnectionKeepAliveState?.() || "{}");
      if (state.terminalCode === 4409 && usb) { expireUsb(); return; }
      if (state.terminalCode === 4003) {
        forget();
        message.value = t('连接已过期，请输入电脑上的配对码');
        return;
      }
    } catch {}
    if (typeof navigator !== "undefined" && typeof navigator.onLine === "boolean")
      networkOffline.value = !usb && !navigator.onLine;
    suspended = false;
    setNetworkListeners(true);
    if (phase.value === "paused" || phase.value === "replaced" || phase.value === "failed") {
      phase.value = "initial";
      attempt = 0;
    }
    if (!socket || socket.readyState >= WebSocket.CLOSING) connect();
  }
  function forget() {
    suspend();
    saveToken("");
    status.value = "unpaired";
    message.value = t('已忘记本机凭证，可重新配对');
  }
  async function request<T>(path: string, init: RequestInit, read: (response: Response) => Promise<T>) {
    if (usbExpired.value) throw new Error(t('USB 连接已失效，请在电脑「偏好设置 → USB 有线连接」中重新连接手机'));
    if (!token) throw new Error(t('需要先与电脑配对'));
    const headers = new Headers(init.headers);
    const generation = credentialGeneration;
    headers.set("Authorization", `Bearer ${token}`);
    if (usbAccess) headers.set("X-PlumeLink-USB",usbAccess);
    const controller = new AbortController();
    const abortParent = () => controller.abort();
    const timeout = setTimeout(() => controller.abort(), 12_000);
    if (init.signal) {
      if (init.signal.aborted) controller.abort();
      else init.signal.addEventListener("abort", abortParent, { once: true });
    }
    try {
      const response = await fetch(path, { ...init, headers, signal: controller.signal });
      if (generation !== credentialGeneration) throw new Error(t('连接已变化，请重试'));
      if (response.status === 410 && usb) { expireUsb(); throw new Error(t('USB 连接已失效，请在电脑「偏好设置 → USB 有线连接」中重新连接手机')); }
      if (response.status === 401) {
        forget();
        throw new Error(t('配对已失效，请重新配对'));
      }
      const result = await read(response);
      if (generation !== credentialGeneration) throw new Error(t('连接已变化，请重试'));
      return result;
    } catch (error) {
      if (controller.signal.aborted && !init.signal?.aborted)
        throw new Error(t('请求电脑超时，请检查连接后重试'));
      throw error;
    } finally {
      clearTimeout(timeout);
      init.signal?.removeEventListener("abort", abortParent);
    }
  }
  function api(path: string, init: RequestInit = {}) {
    return request(path, init, async response => {
      // Keep cancellation and the timeout active until the response body arrives.
      await response.clone().arrayBuffer();
      return response;
    });
  }
  async function requestControl() {
    if (controlRequestPending.value || status.value !== "connected" || controlMode.value !== "primary" || primary.value || !permissions.value.input) return;
    const generation = ++requestGeneration;
    controlRequestPending.value = true;
    // Ignore heartbeats sent before the PC accepted this request.
    requestAcknowledgedAt = Infinity;
    try {
      const response = await api("/api/control/request", { method: "POST" });
      if (generation !== requestGeneration) return;
      if (!response.ok) throw new Error((await response.json()).error || t('无法提交主控请求'));
      waitForControl();
    } catch (error) {
      if (generation !== requestGeneration) return;
      clearControlRequest();
      throw error;
    }
  }
  async function listWindows(signal?: AbortSignal): Promise<RemoteWindow[]> {
    const result = await request("/api/windows", { signal }, async response => {
      const data = await response.json();
      if (!response.ok || !Array.isArray(data.windows)) throw new Error(data.error || t("无法获取窗口列表"));
      return data.windows as unknown[];
    });
    const seen = new Set<number>();
    return result.flatMap((entry): RemoteWindow[] => {
      if (!entry || typeof entry !== "object") return [];
      const window = entry as Record<string, unknown>;
      if (typeof window.hwnd !== "number" || !Number.isSafeInteger(window.hwnd) || window.hwnd <= 0 ||
        seen.has(window.hwnd) || typeof window.title !== "string" || !window.title.trim() || window.title.length > 2048) return [];
      seen.add(window.hwnd);
      return [{
        hwnd: window.hwnd, title: window.title,
        app: typeof window.app === "string" && window.app.length <= 512 ? window.app : "",
        minimized: window.minimized === true,
        icon: thumbnail(window.icon, 128) ?? null,
      }];
    });
  }
  async function switchWindow(hwnd: number) {
    await request("/api/windows/switch", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ hwnd }),
    }, async response => {
      if (response.ok) return;
      const result = await response.json().catch(() => ({}));
      throw new Error(result.error || t("切换窗口失败"));
    });
  }
  return { status, message, usbExpired: computed(() => usbExpired.value), connectionLabel, connectionBadgeLabel, latency, jitter, quality, qualityLabel, features, permissions, fileAccessPending, deviceName, controlMode, primary, connectedCount, primaryName, controlRequestPending, adminGranted, recentController, qft2, transportStats, setResponseMode, setBackgroundAllowed, sendMotion, clearMotion, requestControl, listWindows, switchWindow, pair, send, releaseInput, suspend, resume, forget, api, token: () => token };
}
