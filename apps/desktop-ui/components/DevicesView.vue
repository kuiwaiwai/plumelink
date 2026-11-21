<script setup lang="ts">
import { nextTick, onUnmounted, ref } from "vue";
import { Smartphone, Pencil, Trash2, ShieldCheck } from "@lucide/vue";
import { t, locale } from "../../../shared/i18n";
import type { DesktopState } from "../../../shared/protocol";

const props = defineProps<{
  state: DesktopState;
  busy: boolean;
}>();

const emit = defineEmits<{
  addDevice: [];
  deviceAction: [action: string, id: string, name?: string, permissions?: DesktopState["devices"][number]["permissions"], onDone?: (ok: boolean) => void];
  adminAction: [action: "start" | "stop"];
}>();

const editingDevice = ref("");
const editingName = ref("");
const revokeDeviceConfirm = ref("");
let revokeTimer: ReturnType<typeof setTimeout> | undefined;

async function editDevice(device: DesktopState["devices"][number]) {
  editingDevice.value = device.id;
  editingName.value = device.name;
  await nextTick();
  const input = document.querySelector<HTMLInputElement>(".device-rename input");
  input?.focus();
  input?.select();
}

async function cancelDeviceName() {
  const id = editingDevice.value;
  editingDevice.value = "";
  await nextTick();
  document.getElementById(`rename-${id}`)?.focus();
}

function saveDeviceName() {
  if (!editingDevice.value || !editingName.value.trim()) return;
  emit("deviceAction", "rename", editingDevice.value, editingName.value.trim(), undefined, ok => {
    if (ok) editingDevice.value = "";
  });
}

function setPermission(device: DesktopState["devices"][number], key: keyof typeof device.permissions, value: boolean) {
  emit("deviceAction", "permissions", device.id, undefined, { ...device.permissions, [key]: value });
}

function setControlMode(id?: string) {
  emit("deviceAction", id ? "primary" : "free", id || "");
}

function revokeOne(id: string) {
  if (revokeDeviceConfirm.value !== id) {
    revokeDeviceConfirm.value = id;
    clearTimeout(revokeTimer);
    revokeTimer = setTimeout(() => { revokeDeviceConfirm.value = ""; }, 5000);
    return;
  }
  revokeDeviceConfirm.value = "";
  emit("deviceAction", "revoke", id);
}

function displayTime(value: number) {
  return value
    ? new Date(value * 1000).toLocaleString(locale.value, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })
    : t("旧版记录");
}

onUnmounted(() => clearTimeout(revokeTimer));
</script>

<template>
  <section id="desktop-devices" class="workspace-card" :aria-busy="busy">
    <div class="workspace-heading">
      <h2>{{ t('已绑定的设备') }}</h2>
      <button class="text-button" @click="emit('addDevice')">{{ t('添加新设备') }}</button>
    </div>
    <div class="details-body device-manager">
      <div class="control-mode">
        <span><b>{{ t('多人控制') }}</b><small>{{ state.controlMode === 'free' ? t('自由模式：所有授权设备可同时操作') : t('主控模式：仅指定设备可操作') }}</small></span>
        <button v-if="state.controlMode !== 'free'" :disabled="busy" @click="setControlMode()">{{ t('切换为自由模式') }}</button>
      </div>
      <p v-if="!state.devices.length" class="hint">{{ t('还没有绑定设备。回到连接工作台，用手机扫码添加第一台设备。') }}</p>
      <article v-for="device in state.devices" :key="device.id" class="device-card">
        <header>
          <span class="device-symbol"><Smartphone aria-hidden="true" /></span>
          <span class="device-main">
            <span v-if="editingDevice !== device.id"><b>{{ device.name }}</b><small>{{ device.online ? t('当前在线') : t('最近连接：{0}', displayTime(device.lastSeen)) }}</small></span>
            <span v-else class="device-rename">
              <input v-model="editingName" maxlength="40" :aria-label="t('设备名称')" :disabled="busy" @keyup.enter="saveDeviceName" @keyup.esc="cancelDeviceName" />
              <button :disabled="busy || !editingName.trim()" @click="saveDeviceName">{{ t('保存') }}</button>
              <button :disabled="busy" @click="cancelDeviceName">{{ t('取消') }}</button>
            </span>
          </span>
          <span class="online-dot" :class="{ online: device.online }" :title="device.online ? t('在线') : t('离线')"></span>
          <button :id="`rename-${device.id}`" class="icon-button" :disabled="busy" :aria-label="t('重命名 {0}', device.name)" @click="editDevice(device)"><Pencil aria-hidden="true" /></button>
        </header>
        <div class="device-permissions">
          <span><ShieldCheck aria-hidden="true" />{{ t('权限') }}</span>
          <label><input type="checkbox" :disabled="busy" :checked="device.permissions.input" @change="setPermission(device, 'input', ($event.target as HTMLInputElement).checked)" />{{ t('遥控') }}</label>
          <label><input type="checkbox" :disabled="busy" :checked="device.permissions.text" @change="setPermission(device, 'text', ($event.target as HTMLInputElement).checked)" />{{ t('文字') }}</label>
          <label><input type="checkbox" :disabled="busy" :checked="device.permissions.admin" @change="setPermission(device, 'admin', ($event.target as HTMLInputElement).checked)" />{{ t('管理员控制') }}</label>
          <label><input type="checkbox" :disabled="busy" :checked="device.permissions.files" @change="setPermission(device, 'files', ($event.target as HTMLInputElement).checked)" />{{ t('文件') }}</label>
          <label><input type="checkbox" :disabled="busy" :checked="device.permissions.power" @change="setPermission(device, 'power', ($event.target as HTMLInputElement).checked)" />{{ t('关机') }}</label>
          <label><input type="checkbox" :disabled="busy" :checked="device.permissions.microphone" @change="setPermission(device, 'microphone', ($event.target as HTMLInputElement).checked)" />{{ t('麦克风') }}</label>
        </div>
        <div class="device-actions">
          <button v-if="state.primaryDevice !== device.id" :disabled="busy" @click="setControlMode(device.id)">{{ t('设为主控') }}</button>
          <span v-else class="primary-badge">{{ t('当前主控') }}</span>
          <button class="device-revoke" :disabled="busy" :class="{ armed: revokeDeviceConfirm === device.id }" @click="revokeOne(device.id)"><Trash2 aria-hidden="true" />{{ revokeDeviceConfirm === device.id ? t('再次点击确认撤销') : t('撤销此设备') }}</button>
        </div>
      </article>
      <div v-if="state.devices.some(device => device.permissions.admin)" class="control-mode">
        <span><b>{{ t('管理员辅助进程') }}</b><small>{{ state.admin.active ? t('已就绪，按设备权限控制管理员窗口。') : t('需要在电脑上确认 Windows 授权。') }}</small></span>
        <button :disabled="busy || !state.running" @click="emit('adminAction', state.admin.active ? 'stop' : 'start')">{{ state.admin.active ? t('暂停管理员控制') : t('请求 Windows 授权') }}</button>
      </div>
    </div>
  </section>
</template>
