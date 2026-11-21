import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import QRCode from "qrcode";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { t } from "../../../shared/i18n";
import { errorText, formatHost } from "../../../shared/format";
import type { DesktopState } from "../../../shared/protocol";
import { DEFAULT_SERVICE_PORT, desktop } from "../bridge";

export function useDesktop() {
  const state = ref<DesktopState>({
    running: false,
    discovery: { active: false, error: "" },
    halo: { enabled: true, size: "medium" },
    autostart: false,
    trusted: false,
    pairedCount: 0,
    devices: [],
    controlMode: "free",
    primaryDevice: null,
    controlRequest: null,
    connected: false,
    connectedCount: 0,
    recentController: null,
    code: "",
    codeRemaining: 0,
    pairingIntervalMinutes: 15,
    addresses: [],
    selected: "",
    port: DEFAULT_SERVICE_PORT,
    error: "",
    receivePath: "",
    sharedFiles: [],
    admin: { active: false },
  });

  const selected = ref("");
  const haloSize = ref<DesktopState["halo"]["size"]>("medium");
  const savingHaloSize = ref(false);
  let queuedHaloSize: DesktopState["halo"]["size"] | null = null;
  const qr = ref("");
  const busy = ref(false);
  const initializing = ref(true);
  const operationError = ref("");
  const connectionError = ref("");
  const copyMessage = ref("");
  const draggingFiles = ref(false);
  let disposed = false;
  let feedbackTimer: ReturnType<typeof setTimeout> | undefined;
  let unsubscribe = () => {};
  let unlistenDrop = () => {};

  const url = computed(() =>
    state.value.selected
      ? `https://${formatHost(state.value.selected)}:${state.value.port}`
      : "",
  );

  function notify(message: string) {
    clearTimeout(feedbackTimer);
    copyMessage.value = message;
    feedbackTimer = setTimeout(() => { copyMessage.value = ""; }, 5000);
  }

  function update(value: DesktopState) {
    if (disposed) return;
    initializing.value = false;
    connectionError.value = "";
    state.value = value;
    if (!savingHaloSize.value) haloSize.value = value.halo.size;
    if (!selected.value || !value.addresses.some(a => a.address === selected.value)) {
      selected.value = value.selected;
    }
  }

  watch(
    [url, () => state.value.running, () => state.value.code, () => state.value.pairingIntervalMinutes === 0 || state.value.codeRemaining > 0],
    async (_, __, onCleanup) => {
      let cancelled = false;
      onCleanup(() => { cancelled = true; });
      qr.value = "";
      if (!state.value.running || !url.value || (state.value.pairingIntervalMinutes !== 0 && state.value.codeRemaining <= 0) || !/^[0-9]{6}$/.test(state.value.code)) return;
      try {
        const payload = `plumelink://connect?address=${encodeURIComponent(`${formatHost(state.value.selected)}:${state.value.port}`)}&code=${state.value.code}`;
        const generated = await QRCode.toDataURL(payload, {
          width: 320,
          margin: 4,
          errorCorrectionLevel: "M",
          color: { dark: "#203653", light: "#ffffff" },
        });
        if (!cancelled) qr.value = generated;
      } catch {
        if (!cancelled) operationError.value = t("二维码生成失败，请使用连接地址添加电脑");
      }
    },
  );

  async function action(name: string) {
    if (busy.value) return;
    operationError.value = "";
    busy.value = true;
    try {
      const value = await desktop.action(name, selected.value);
      if (value) update(value);
    } catch (e) {
      operationError.value = errorText(e, t("操作失败"));
    } finally {
      busy.value = false;
    }
  }

  async function setPort(port: number) {
    if (busy.value) return;
    busy.value = true;
    try {
      update(await desktop.setPort(port, selected.value));
      notify(t("服务端口已更新"));
    } finally {
      busy.value = false;
    }
  }

  async function saveHaloSize() {
    queuedHaloSize = haloSize.value;
    if (savingHaloSize.value) return;
    savingHaloSize.value = true;
    operationError.value = "";
    try {
      // Serialize quick selections without disabling or repainting every control.
      while (queuedHaloSize !== null) {
        const size = queuedHaloSize;
        queuedHaloSize = null;
        try { update(await desktop.action(`halo${size}`, selected.value)); }
        catch (error) { operationError.value = errorText(error, t("操作失败")); }
      }
    } finally {
      savingHaloSize.value = false;
      haloSize.value = state.value.halo.size;
    }
  }

  async function transfer(name: string, id?: number, ids?: number[]) {
    if (busy.value) return;
    operationError.value = "";
    busy.value = true;
    try {
      update(await desktop.transfer(name, id, ids));
    } catch (e) {
      operationError.value = errorText(e, t("操作失败"));
    } finally {
      busy.value = false;
    }
  }

  async function deviceAction(
    name: string,
    id: string,
    deviceName?: string,
    permissions?: DesktopState["devices"][number]["permissions"],
  ) {
    if (busy.value) return false;
    operationError.value = "";
    busy.value = true;
    try {
      update(await desktop.device(name, id, deviceName, permissions));
      return true;
    } catch (e) {
      operationError.value = errorText(e, t("操作失败"));
      return false;
    } finally {
      busy.value = false;
    }
  }

  async function adminAction(name: "start" | "stop") {
    if (busy.value) return;
    operationError.value = "";
    busy.value = true;
    try {
      update(await desktop.admin(name));
    } catch (e) {
      operationError.value = errorText(e, t("操作失败"));
    } finally {
      busy.value = false;
    }
  }

  onMounted(async () => {
    busy.value = true;
    try { update(await desktop.state()); }
    catch {
      initializing.value = false;
      connectionError.value = t("暂时无法读取电脑服务状态，正在自动重试…");
    } finally {
      busy.value = false;
    }
    if (disposed) return;
    unsubscribe = desktop.subscribe(
      value => { if (!busy.value) update(value); },
      () => { connectionError.value = t("服务状态暂时不可用，正在自动重试…"); },
    );
    try {
      unlistenDrop = await getCurrentWebview().onDragDropEvent(async event => {
        draggingFiles.value = event.payload.type === "enter" || event.payload.type === "over";
        if (event.payload.type === "drop") {
          draggingFiles.value = false;
          if (busy.value) return;
          busy.value = true;
          try {
            update(await desktop.sharePaths(event.payload.paths));
            notify(t("所选文件已加入共享列表"));
          } catch (error) {
            operationError.value = errorText(error, t("操作失败"));
          } finally {
            busy.value = false;
          }
        }
        if (event.payload.type === "leave") draggingFiles.value = false;
      });
    } catch {}
  });

  onUnmounted(() => {
    disposed = true;
    unsubscribe();
    unlistenDrop();
    clearTimeout(feedbackTimer);
  });

  return {
    state,
    selected,
    haloSize,
    savingHaloSize,
    qr,
    url,
    busy,
    initializing,
    operationError,
    connectionError,
    copyMessage,
    draggingFiles,
    notify,
    action,
    saveHaloSize,
    setPort,
    transfer,
    deviceAction,
    adminAction,
  };
}
