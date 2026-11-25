<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref } from "vue";
import { FileUp, X, Activity, LayoutDashboard, Monitor, Settings2, CircleHelp, Network } from "@lucide/vue";
import { displayMessage, t } from "../../shared/i18n";
import type { DesktopState } from "../../shared/protocol";
import { homepage, version } from "../../package.json";
import { isTauri, invoke } from "@tauri-apps/api/core";
import "./style.css";
import { useDesktop } from "./composables/useDesktop";
import WindowTitlebar from "./components/WindowTitlebar.vue";
import OverviewView from "./components/OverviewView.vue";
import DevicesView from "./components/DevicesView.vue";
import TransferView from "./components/TransferView.vue";
import SettingsView from "./components/SettingsView.vue";
import DiagnosticsView from "./components/DiagnosticsView.vue";

const {
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
  receiveAction,
  deviceAction,
  adminAction,
} = useDesktop();

const showPairing = ref(false);
const activeSection = ref("overview");
const contentElement = ref<HTMLElement | null>(null);
const diagnosticsView = ref<{ focusPanel?: () => void } | null>(null);
const revokeAllConfirm = ref(false);
let revokeAllTimer: ReturnType<typeof setTimeout> | undefined;

const pages = computed<Record<string, string>>(() => ({
  overview: t("连接工作台"),
  devices: t("设备管理"),
  transfer: t("文件传输"),
  settings: t("偏好设置"),
  diagnostics: t("帮助与诊断"),
}));

function navigate(section: string) {
  activeSection.value = section;
  contentElement.value?.scrollTo({ top: 0, behavior: "auto" });
}

async function openDiagnostics() {
  navigate("diagnostics");
  await nextTick();
  diagnosticsView.value?.focusPanel?.();
}

async function openProjectPage(event: MouseEvent) {
  if (!isTauri()) return;
  event.preventDefault();
  try {
    await invoke("open_project_page");
  } catch {
    operationError.value = t("无法打开项目链接");
  }
}

function revokeAll() {
  if (!revokeAllConfirm.value) {
    revokeAllConfirm.value = true;
    clearTimeout(revokeAllTimer);
    revokeAllTimer = setTimeout(() => { revokeAllConfirm.value = false; }, 5000);
    return;
  }
  revokeAllConfirm.value = false;
  void action("disconnect");
}

async function handleDeviceAction(
  name: string,
  id: string,
  deviceName?: string,
  permissions?: DesktopState["devices"][number]["permissions"],
  onDone?: (ok: boolean) => void,
) {
  const ok = await deviceAction(name, id, deviceName, permissions);
  onDone?.(ok);
}

onUnmounted(() => clearTimeout(revokeAllTimer));
</script>

<template>
  <WindowTitlebar @error="notify" />
  <main class="desktop-shell" :class="{ 'dragging-files': draggingFiles }" :aria-busy="busy || initializing">
    <div v-if="draggingFiles" class="drop-overlay"><FileUp aria-hidden="true" /><b>{{ t('释放以共享到手机') }}</b><span>{{ t('仅添加普通文件，单文件最大 2 GB') }}</span></div>
    <aside class="desktop-sidebar">
      <div class="sidebar-status" :class="{ online: state.running && state.connected }"><span class="dot" :class="{ live: state.running && state.connected }"></span><span>{{ state.running && state.connected ? t('电脑已连接') : state.running ? t('等待设备连接') : t('服务已停止') }}</span></div>
      <nav :aria-label="t('桌面导航')">
        <button :title="t('总览')" :aria-current="activeSection === 'overview' ? 'page' : undefined" :class="{ active: activeSection === 'overview' }" @click="navigate('overview')"><LayoutDashboard aria-hidden="true" />{{ t('总览') }}</button>
        <button :title="t('设备管理')" :aria-current="activeSection === 'devices' ? 'page' : undefined" :class="{ active: activeSection === 'devices' }" @click="navigate('devices')"><Monitor aria-hidden="true" />{{ t('设备管理') }}<span v-if="state.connectedCount" class="nav-count">{{ state.connectedCount }}</span></button>
        <button :title="t('文件传输')" :aria-current="activeSection === 'transfer' ? 'page' : undefined" :class="{ active: activeSection === 'transfer' }" @click="navigate('transfer')"><Network aria-hidden="true" />{{ t('文件传输') }}</button>
        <button :title="t('偏好设置')" :aria-current="activeSection === 'settings' ? 'page' : undefined" :class="{ active: activeSection === 'settings' }" @click="navigate('settings')"><Settings2 aria-hidden="true" />{{ t('偏好设置') }}</button>
      </nav>
      <div class="sidebar-bottom">
        <button :title="t('连接帮助')" :aria-current="activeSection === 'diagnostics' ? 'page' : undefined" :class="{ active: activeSection === 'diagnostics' }" @click="navigate('diagnostics')"><CircleHelp aria-hidden="true" />{{ t('连接帮助') }}</button>
        <div class="sidebar-version"><span>v{{ version }}</span><a :href="homepage" target="_blank" rel="noopener noreferrer" @click="openProjectPage">PlumeLink</a></div>
      </div>
    </aside>
    <div ref="contentElement" class="desktop-content">
      <header class="topbar">
        <div><h1>{{ pages[activeSection] }}</h1></div>
        <div class="topbar-actions">
          <span class="connection-pill" :class="{ online: state.running && state.connected }"><span class="dot" :class="{ live: state.running && state.connected }"></span>{{ state.running && state.connected ? t('{0} 台设备在线', state.connectedCount) : state.running ? t('等待设备连接') : t('服务已停止') }}</span>
          <button class="topbar-icon" :aria-label="t('打开诊断')" :title="t('打开诊断')" @click="openDiagnostics"><Activity aria-hidden="true" /></button>
        </div>
      </header>
      <p v-if="connectionError" class="error" role="status">{{ displayMessage(connectionError) }}</p>
      <div v-if="operationError" class="error operation-error" role="alert"><span>{{ displayMessage(operationError) }}</span><button :aria-label="t('关闭错误提示')" @click="operationError = ''"><X aria-hidden="true" /></button></div>
      <section v-if="state.controlRequest" class="control-request" aria-live="assertive"><div><b>{{ state.controlRequest.name }} {{ t('请求成为主控') }}</b><p>{{ t('批准后将切换到主控模式，其他设备暂时不能操作。') }}</p></div><button :disabled="busy" @click="deviceAction('deny', state.controlRequest.id)">{{ t('拒绝') }}</button><button class="accent" :disabled="busy" @click="deviceAction('approve', state.controlRequest.id)">{{ t('批准接管') }}</button></section>
      <p v-if="state.error" class="error" role="alert">{{ displayMessage(state.error) }}</p>

      <div v-show="activeSection === 'overview'" class="workspace-view" data-view="overview">
        <OverviewView v-model:show-pairing="showPairing" :state="state" :initializing="initializing" :busy="busy" :qr="qr" :url="url" @action="action" @navigate="navigate" @notify="notify" />
      </div>
      <div v-show="activeSection === 'devices'" class="workspace-view" data-view="devices">
        <DevicesView :state="state" :busy="busy" @add-device="navigate('overview'); showPairing = true" @device-action="handleDeviceAction" @admin-action="adminAction" />
      </div>
      <div v-show="activeSection === 'transfer'" class="workspace-view" data-view="transfer">
        <TransferView :state="state" :busy="busy" :receive-action="receiveAction" @transfer="transfer" />
      </div>
      <div v-show="activeSection === 'settings'" class="workspace-view" data-view="settings">
        <SettingsView v-model:selected="selected" v-model:halo-size="haloSize" :state="state" :busy="busy" :saving-halo-size="savingHaloSize" :set-port="setPort" @action="action" @save-halo-size="saveHaloSize" />
      </div>
      <div v-show="activeSection === 'diagnostics'" class="workspace-view" data-view="diagnostics">
        <DiagnosticsView ref="diagnosticsView" :state="state" :url="url" />
      </div>

      <footer><div><button v-if="state.running" :disabled="busy" @click="action('stop')">{{ t('停止遥控') }}</button><button :class="{ 'danger-confirm': revokeAllConfirm }" :disabled="!state.trusted || busy" @click="revokeAll">{{ revokeAllConfirm ? t('再次点击确认全部撤销') : t('撤销全部设备') }}</button><button :disabled="busy" @click="action('quit')">{{ t('退出程序') }}</button></div></footer>
    </div>
    <div class="feedback-region" role="status" aria-live="polite" aria-atomic="true"><div v-if="copyMessage" class="feedback-toast"><span>{{ displayMessage(copyMessage) }}</span><button :aria-label="t('关闭操作提示')" @click="copyMessage = ''"><X aria-hidden="true" /></button></div></div>
  </main>
</template>
