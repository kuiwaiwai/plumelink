<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Cable, ChevronDown, RefreshCw, Smartphone, ShieldCheck } from "@lucide/vue";
import { t, displayMessage } from "../../../shared/i18n";
import { errorText } from "../../../shared/format";
type UsbStatus = { available: boolean; busy: boolean; activeCount: number; endpoint: string; endpoints?: string[]; fingerprint: string; code: string; error: string; devices: { id: string; name: string; state: string; linked: boolean; selected: boolean; error: string; endpoint?: string; connectionUnverified?: boolean }[] };
const connectionUnverified = computed(() => status.value.devices.some(device => device.connectionUnverified));
const status = ref<UsbStatus>({ available: false, busy: false, activeCount: 0, endpoint: "", fingerprint: "", code: "", error: "", devices: [] });
const busy = ref(false), loading = ref(false), open = ref(false), error = ref("");
const detectionError = ref("");
const pendingAction = ref(""), cancelling = ref(false), notice = ref("");
const pendingHint = computed(() => cancelling.value ? t("正在取消 USB 连接…")
  : pendingAction.value === "connect" ? t("正在处理 USB 连接，请留意手机上的确认提示…")
  : pendingAction.value === "disconnect" ? t("正在断开 USB 连接…") : "");
let disposed = false, revision = 0;
let refreshQueued = false, manualRefreshQueued = false;
const failedDevice = ref("");
const displayedError = computed(() => {
  const inDevice = failedDevice.value && status.value.devices.some(device => device.id === failedDevice.value && device.error === error.value);
  return (inDevice ? "" : error.value) || detectionError.value || status.value.error;
});
async function refresh(manual = false) {
  if (disposed || busy.value || !open.value || document.hidden) return;
  if (loading.value) { refreshQueued = true; manualRefreshQueued ||= manual; return; }
  loading.value = true;
  const request = revision;
  try {
    const next = await invoke<UsbStatus>("usb_action", { action: "status" });
    if (!disposed && request === revision) {
      status.value = next; detectionError.value = "";
      if (manual || next.devices.some(device => device.id === failedDevice.value && device.linked)) { error.value = ""; failedDevice.value = ""; }
    }
  } catch (reason) {
    if (!disposed && request === revision) detectionError.value = errorText(reason, t("USB 检测失败，请重试"));
  } finally {
    loading.value = false;
    if (refreshQueued && !disposed) {
      const manual = manualRefreshQueued;
      refreshQueued = false; manualRefreshQueued = false;
      void refresh(manual);
    }
  }
}
async function action(action: string, id?: string) {
  if (busy.value && action !== "cancel") return;
  if (action === "cancel") {
    if (pendingAction.value !== "connect" || cancelling.value) return;
    cancelling.value = true; const request = revision;
    try { await invoke("usb_action", { action }); }
    catch (reason) { if (!disposed && request === revision && busy.value && pendingAction.value === "connect") { cancelling.value = false; error.value = errorText(reason, t("USB 操作失败，请检查手机授权和数据线")); } }
    return;
  }
  refreshQueued = false; manualRefreshQueued = false;
  busy.value = true; pendingAction.value = action; cancelling.value = false; notice.value = ""; error.value = ""; detectionError.value = ""; failedDevice.value = ""; const request = ++revision;
  let failed = false;
  try {
    const next = await invoke<UsbStatus>("usb_action", { action, id });
    if (!disposed && request === revision) { status.value = next; error.value = ""; }
  } catch (reason) {
    failed = true;
    if (!disposed && request === revision) {
      error.value = "";
      const message = errorText(reason, t("USB 操作失败，请检查手机授权和数据线"));
      if (displayMessage(message) === t("USB 连接已取消")) notice.value = t("USB 连接已取消");
      else { error.value = message; if (action === "connect") failedDevice.value = id || ""; }
    }
  } finally {
    if (!disposed && request === revision) {
      busy.value = false; pendingAction.value = ""; cancelling.value = false;
      if (failed) void refresh();
    }
  }
}
function toggle(event: Event) { open.value = (event.target as HTMLDetailsElement).open; if (open.value) void refresh(); }
const timer = setInterval(() => { void refresh(); }, 3000);
onUnmounted(() => { disposed = true; revision++; clearInterval(timer); });
const deviceState = (device: UsbStatus["devices"][number]) => device.linked ? device.connectionUnverified ? t("调试通道已就绪") : t("USB 通道已就绪")
  : device.selected && device.state === "disconnected" ? t("数据线已断开，重新插接后会自动恢复")
  : device.selected && device.state === "unavailable" ? t("暂时无法检测 USB 通道")
  : device.selected && device.error ? t("USB 通道尚未就绪")
  : device.state === "unauthorized" ? t("请在手机上允许 USB 调试") : device.state === "offline" ? t("手机暂时离线，请重新插接数据线")
  : device.state === "ready" ? t("可以连接") : t("手机尚未准备就绪");
</script>

<template>
  <details id="usb-connection" class="settings usb-settings" @toggle="toggle">
    <summary><span class="section-title"><Cable aria-hidden="true" />{{ t('USB 有线连接') }}</span><ChevronDown aria-hidden="true" /></summary>
    <div class="details-body usb-body">
      <p v-if="!connectionUnverified" class="hint">{{ t('用数据线连接手机，在手机开发者选项中开启 USB 调试，并允许这台电脑。连接后，遥控、文字、文件和麦克风均通过数据线传输。') }}</p>
      <div class="usb-tools">
        <button :disabled="busy || loading" @click="refresh(true)"><RefreshCw aria-hidden="true" />{{ loading ? t('正在检测…') : t('重新检测') }}</button>
        <button v-if="status.activeCount" class="usb-disconnect-all" :disabled="busy" @click="action('disconnect')">{{ t('断开全部 USB 连接') }}</button>
        <button v-if="pendingAction === 'connect'" :disabled="cancelling" @click="action('cancel')">{{ t('取消') }}</button>
      </div>
      <p v-if="connectionUnverified" class="hint">{{ t('当前组件无法区分连接类型，列表可能包含无线设备和模拟器。请选择通过数据线连接的手机。') }}</p>
      <p v-if="displayedError" class="error" role="alert">{{ displayMessage(displayedError) }}</p>
      <p v-if="notice" class="hint" role="status">{{ displayMessage(notice) }}</p>
      <p v-else-if="!displayedError && open && !loading && !busy && !status.devices.length" class="hint" role="status">{{ t('未发现 USB 手机。请检查数据线、USB 调试授权及手机 USB 驱动。') }}</p>
      <ul v-if="status.devices.length" class="usb-devices">
        <li v-for="device in status.devices" :key="device.id">
          <Smartphone aria-hidden="true" />
          <div class="usb-device-copy"><b>{{ device.name }}</b><small>{{ deviceState(device) }}</small><small v-if="device.endpoint" class="usb-endpoint">{{ device.endpoint }}</small><small v-if="device.error" class="error" role="status">{{ displayMessage(device.error) }}</small></div>
          <button :disabled="busy || (!device.selected && device.state !== 'ready')" :class="{ accent: !device.selected }" @click="action(device.selected ? 'disconnect' : 'connect', device.id)">{{ device.selected ? t('断开') : t('连接') }}</button>
        </li>
      </ul>
      <div v-if="status.endpoint" class="usb-pairing">
        <div><span>{{ t('6 位配对码') }}</span><strong>{{ status.code }}</strong></div>
        <details class="usb-certificate"><summary><ShieldCheck aria-hidden="true" /><span>{{ t('核对电脑证书') }}</span><ChevronDown aria-hidden="true" /></summary><code v-for="endpoint in status.endpoints?.length ? status.endpoints : [status.endpoint]" :key="endpoint">{{ endpoint }}</code><code>{{ status.fingerprint }}</code></details>
      </div>
      <p class="hint">{{ t('首次连接仍需在手机确认电脑并输入配对码。USB 调试授权不会代替羽连的设备权限。') }}</p>
      <p v-if="pendingHint" class="hint" role="status">{{ pendingHint }}</p>
    </div>
  </details>
</template>

<style scoped>
.usb-body { display: grid; gap: 14px; }
.usb-body > p { margin: 0; overflow-wrap: anywhere; }
.usb-tools { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
.usb-tools .text-button { color: var(--muted); }
.usb-disconnect-all { margin-inline-start: auto; }
.usb-devices { list-style: none; padding: 0; margin: 0; border: 1px solid var(--edge); border-radius: 14px; overflow: hidden; }
.usb-devices li { display: flex; align-items: center; gap: 12px; padding: 14px; }
.usb-devices li + li { border-top: 1px solid var(--edge); }
.usb-devices li > svg { flex: 0 0 20px; width: 20px; height: 20px; color: var(--muted); }
.usb-device-copy { display: grid; flex: 1; min-width: 0; gap: 5px; overflow-wrap: anywhere; }
.usb-device-copy small { color: var(--muted); line-height: 1.5; }
.usb-device-copy small.error { margin: 0; padding: 0; background: none; color: var(--danger-text); }
.usb-endpoint { font-variant-numeric: tabular-nums; }
.usb-devices button { flex: none; }
.usb-pairing { display: flex; align-items: flex-start; justify-content: space-between; flex-wrap: wrap; gap: 12px 24px; padding: 16px; border: 1px solid var(--edge); border-radius: 12px; background: var(--surface-2); }
.usb-pairing > div { display: grid; gap: 4px; flex: 0 0 auto; }
.usb-pairing > div > span { font-size: 12px; }
.usb-pairing strong { font-size: 24px; letter-spacing: .16em; font-variant-numeric: tabular-nums; }
.usb-certificate { flex: 0 1 60%; min-width: 0; }
.usb-certificate > summary { min-height: 32px; padding: 0; justify-content: flex-end; color: var(--text); font-size: 13px; font-weight: 500; border: 0; }
.usb-certificate > summary:hover { background: transparent; color: var(--accent); }
.usb-pairing code { display: block; margin-top: 10px; overflow-wrap: anywhere; font-size: 12px; user-select: text; }
@container preferences (max-width: 540px) {
  .usb-disconnect-all { margin-inline-start: 0; }
  .usb-pairing { flex-direction: column; }
  .usb-certificate { width: 100%; flex-basis: auto; }
  .usb-certificate > summary { justify-content: flex-start; }
}
</style>
