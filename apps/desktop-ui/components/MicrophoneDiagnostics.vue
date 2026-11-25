<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { displayMessage, t } from "../../../shared/i18n";
import { errorText } from "../../../shared/format";

type Output = { id: string; name: string; virtualHint: boolean };
type Input = { id: string; name: string };
type EndpointVolume = { id: string; volumePercent: number | null; muted: boolean | null; error: string };
type EndpointVolumes = { output: EndpointVolume | null; recording: EndpointVolume | null };
type Probe = {
  id: string;
  name: string;
  detected: boolean;
  windows: number;
  matches: number;
  peak: number;
  error: string;
};
type ActionValue = {
  outputs: Output[];
  recordingInputs: Input[];
  selected: string;
  probes?: Probe[];
  target?: Input | null;
  defaultName?: string;
  restoreAvailable?: boolean;
  restoredRoles?: number;
  verifiedInput?: Input | null;
};

defineProps<{
  status?: {
    active: boolean;
    deviceName: string;
    endpointName: string;
    receivedFrames?: number;
    receivedSignalFrames?: number;
    renderedSignalSamples?: number;
  };
}>();

const outputs = ref<Output[]>([]);
const selected = ref("");
const recordingInputs = ref<Input[]>([]);
const probes = ref<Probe[]>([]);
const target = ref<Input | null>(null);
const verifiedInput = ref<Input | null>(null);
const volumes = ref<EndpointVolumes | null>(null);
const volumeBusy = ref(false);
const volumeMessage = ref("");
const checked = ref(false);
const canRestore = ref(false);
const busy = ref(false);
const message = ref("");
const notice = ref("");
let refreshPending = false;

const selectedName = computed(
  () => outputs.value.find(output => output.id === selected.value)?.name || t("未选择"),
);
const recording = computed(() => target.value ?? verifiedInput.value);
const volumeRows = computed(() => [
  { key: "output" as const, label: t("播放端"), id: selected.value, name: selected.value ? selectedName.value : "" },
  { key: "recording" as const, label: t("录音端"), id: recording.value?.id ?? "", name: recording.value?.name ?? "" },
].map(row => {
  const raw = volumes.value?.[row.key];
  const percent = raw?.volumePercent;
  const level = raw && {
    ...raw,
    volumePercent: typeof percent === "number" && Number.isInteger(percent) && percent >= 0 && percent <= 100 ? percent : null,
    muted: typeof raw.muted === "boolean" ? raw.muted : null,
  };
  return { ...row, level: level?.id === row.id ? level : null };
}));
function volumeText(row: typeof volumeRows.value[number]) {
  if (!row.id) return [t(row.key === "output" ? "未选择" : "尚未确认")];
  if (volumeBusy.value) return [t("正在读取…")];
  const value = row.level;
  if (!value || (value.volumePercent === null && value.muted === null)) return [t("无法读取")];
  const parts = value.volumePercent === null ? [t("音量未知")] : [t("音量 {0}%", value.volumePercent)];
  if (value.muted === true) parts.push(t("已静音"));
  else if (value.muted === null) parts.push(t("静音状态未知"));
  return parts;
}
const volumeHint = computed(() => {
  if (volumeBusy.value) return "";
  if (volumeRows.value.some(row => row.level?.muted === true || row.level?.volumePercent === 0))
    return t("请在 Windows 声音设置中取消静音或调高对应设备的音量。");
  const error = volumeMessage.value || volumeRows.value.find(row => row.level?.error)?.level?.error;
  if (error) return displayMessage(error);
  return selected.value && !recording.value ? t("先检测通路，确认收音软件应选择的录音设备。") : "";
});

async function readVolumes() {
  volumeBusy.value = true;
  volumeMessage.value = "";
  try {
    volumes.value = await invoke<EndpointVolumes>("microphone_action", {
      action: "levels", id: selected.value, recording: recording.value?.id ?? "",
    });
  } catch (error) {
    volumes.value = null;
    volumeMessage.value = errorText(error, t("无法读取设备音量，请刷新设备状态。"));
  } finally {
    volumeBusy.value = false;
  }
}

async function action(actionName: string, id = selected.value) {
  if (busy.value) return;
  busy.value = true;
  volumeBusy.value = true;
  message.value = "";
  if (actionName === "check") {
    checked.value = false;
    probes.value = [];
    target.value = null;
  }
  try {
    const value = await invoke<ActionValue>("microphone_action", { action: actionName, id });
    outputs.value = value.outputs;
    recordingInputs.value = value.recordingInputs;
    if (selected.value !== value.selected) { target.value = null; verifiedInput.value = null; volumes.value = null; }
    selected.value = value.selected;
    if (value.verifiedInput !== undefined) verifiedInput.value = value.verifiedInput;
    if (target.value && !recordingInputs.value.some(input => input.id === target.value?.id)) target.value = null;
    if (verifiedInput.value && !recordingInputs.value.some(input => input.id === verifiedInput.value?.id)) verifiedInput.value = null;
    if (value.probes) {
      probes.value = value.probes;
      target.value = value.target ?? null;
      checked.value = true;
    }
    if (value.defaultName) notice.value = t("已将「{0}」设为默认录音设备", value.defaultName);
    if (value.restoredRoles !== undefined) notice.value = t("已恢复 {0} 个默认角色的原录音设备", value.restoredRoles);
    if (typeof value.restoreAvailable === "boolean") canRestore.value = value.restoreAvailable;
    await readVolumes();
  } catch (error) {
    message.value = errorText(error, t("操作失败"));
    volumes.value = null;
  } finally {
    volumeBusy.value = false;
    busy.value = false;
    if (refreshPending) { refreshPending = false; void action("list"); }
  }
}

onMounted(() => action("list"));
defineExpose({ refresh: () => { if (busy.value) refreshPending = true; else return action("list"); } });
</script>

<template>
  <div class="audio-inputs mic-route">
    <strong>{{ t('手机麦克风通路') }}</strong>
    <p class="hint">
      {{ status?.active ? t('送音中：{0}', status.deviceName) : t('未在使用') }}
    </p>
    <p v-if="status?.active" class="hint">
      {{ t('收到') }} {{ status.receivedFrames ?? 0 }} {{ t('帧 · 非静音') }} {{ status.receivedSignalFrames ?? 0 }} {{ t('帧 · 已提交') }} {{ status.renderedSignalSamples ?? 0 }} {{ t('个非静音样本') }}
    </p>
    <div class="diagnostic-actions">
      <button :disabled="busy || !selected || status?.active" @click="action('check')">{{ t('检测通路') }}</button>
      <button :disabled="busy" @click="action('list')">{{ t('刷新设备状态') }}</button>
      <button v-if="target" :disabled="busy" @click="action('default', target.id)">{{ t('设为默认录音设备') }}</button>
      <button v-if="canRestore" :disabled="busy" @click="action('restore')">{{ t('恢复原默认录音设备') }}</button>
    </div>
    <ul v-if="checked">
      <li v-for="probe in probes" :key="probe.id">
        {{ probe.name }} ·
        {{
          probe.error
            ? t('无法检查：{0}', displayMessage(probe.error))
            : probe.detected
              ? t('检测到测试音（{0}/{1} 窗口，峰值 {2}）', probe.matches, probe.windows, probe.peak)
              : t('未检测到测试音')
        }}
      </li>
    </ul>
    <p v-if="checked" class="hint">
      {{
        target
          ? t('收音软件应选择录音设备「{0}」。', target.name)
          : t('测试音没有到达任何虚拟录音端，请确认线缆驱动正常，或改选另一个虚拟播放端后重新检测。')
      }}
    </p>
    <section class="mic-endpoint-volumes" :aria-busy="volumeBusy" :aria-label="t('Windows 设备音量')">
      <strong>{{ t('Windows 设备音量') }}</strong>
      <dl>
        <div v-for="row in volumeRows" :key="row.key" :data-endpoint="row.key">
          <dt>{{ row.label }}<span v-if="row.name" :title="row.name">{{ row.name }}</span></dt>
          <dd :class="{ 'is-muted': row.level?.muted === true || row.level?.volumePercent === 0 }">
            <span v-for="part in volumeText(row)" :key="part">{{ part }}</span>
          </dd>
        </div>
      </dl>
      <p v-if="volumeHint" class="hint" role="status">{{ volumeHint }}</p>
    </section>
    <p class="hint">{{ t('Windows 可选录音设备') }}</p>
    <ul v-if="recordingInputs.length">
      <li v-for="input in recordingInputs" :key="input.id">{{ input.name }}</li>
    </ul>
    <p v-else class="hint">{{ t('未发现可用录音设备。请安装并启用虚拟音频线缆，再刷新设备。') }}</p>
    <p v-if="notice || message" class="hint" role="status">{{ displayMessage(message || notice) }}</p>
  </div>
</template>
