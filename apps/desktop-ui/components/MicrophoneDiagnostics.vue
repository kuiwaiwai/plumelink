<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { displayMessage, t } from "../../../shared/i18n";
import { errorText } from "../../../shared/format";

type Output = { id: string; name: string; virtualHint: boolean };
type Input = { id: string; name: string };
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
const checked = ref(false);
const canRestore = ref(false);
const busy = ref(false);
const message = ref("");
const notice = ref("");

const selectedName = computed(
  () => outputs.value.find(output => output.id === selected.value)?.name || t("未选择"),
);

async function action(actionName: string, id = selected.value) {
  if (busy.value) return;
  busy.value = true;
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
    selected.value = value.selected;
    if (value.probes) {
      probes.value = value.probes;
      target.value = value.target ?? null;
      checked.value = true;
    }
    if (value.defaultName) notice.value = t("已将「{0}」设为默认录音设备", value.defaultName);
    if (value.restoredRoles !== undefined) notice.value = t("已恢复 {0} 个默认角色的原录音设备", value.restoredRoles);
    if (typeof value.restoreAvailable === "boolean") canRestore.value = value.restoreAvailable;
  } catch (error) {
    message.value = errorText(error, t("操作失败"));
  } finally {
    busy.value = false;
  }
}

onMounted(() => action("list"));
defineExpose({ refresh: () => { if (!busy.value) return action("list"); } });
</script>

<template>
  <div class="audio-inputs mic-route">
    <strong>{{ t('手机麦克风通路') }}</strong>
    <p class="hint">
      {{ status?.active ? t('送音中：{0}', status.deviceName) : t('未在使用') }} {{ t('· 播放端：') }}{{ selectedName }}
    </p>
    <p v-if="status?.active" class="hint">
      {{ t('收到') }} {{ status.receivedFrames ?? 0 }} {{ t('帧 · 非静音') }} {{ status.receivedSignalFrames ?? 0 }} {{ t('帧 · 已提交') }} {{ status.renderedSignalSamples ?? 0 }} {{ t('个非静音样本') }}
    </p>
    <div class="diagnostic-actions">
      <button :disabled="busy || !selected || status?.active" @click="action('check')">{{ t('检测通路') }}</button>
      <button :disabled="busy" @click="action('list')">{{ t('刷新录音设备') }}</button>
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
    <p class="hint">{{ t('Windows 可选录音设备') }}</p>
    <ul v-if="recordingInputs.length">
      <li v-for="input in recordingInputs" :key="input.id">{{ input.name }}</li>
    </ul>
    <p v-else class="hint">{{ t('未发现可用录音设备。请安装并启用虚拟音频线缆，再刷新设备。') }}</p>
    <p v-if="notice || message" class="hint" role="status">{{ displayMessage(message || notice) }}</p>
  </div>
</template>
