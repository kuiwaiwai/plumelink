<script setup lang="ts">
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Mic, ChevronDown } from "@lucide/vue";
import { displayMessage, t } from "../../../shared/i18n";
import { errorText } from "../../../shared/format";

type Output = { id: string; name: string; virtualHint: boolean; plumeLink?: boolean };
type ActionValue = { outputs: Output[]; recordingInputs?: Output[]; cableInstalled?: boolean; selected: string; gainDb?: number; target?: { name: string }; verifiedInput?: { name: string } };

const props = defineProps<{
  status?: {
    active: boolean;
    deviceName: string;
    endpointName: string;
    error: string;
    receivedFrames?: number;
    receivedSignalFrames?: number;
    renderedSignalSamples?: number;
    inputLevel?: number;
    outputLevel?: number;
    clipping?: boolean;
  };
}>();

const outputs = ref<Output[]>([]);
const loaded = ref(false);
const selected = ref("");
const busy = ref(false);
const message = ref("");
const verifiedInput = ref("");
const serviceInstalled = ref(false);
const inputs = ref<Output[]>([]);
const installing = ref(false);
const gainDb = ref(6);
const savedGainDb = ref(6);
const gainBusy = ref(false);
const stopBusy = ref(false);
const gainClipping = computed(() => props.status?.active === true && props.status.clipping === true);
let gainRevision = 0;

const signalState = computed(() => {
  const state = props.status;
  if (!state?.receivedFrames) return t("通道已打开，等待手机音频");
  if (!state.receivedSignalFrames) return t("已收到音频帧，样本均为静音");
  if (!state.renderedSignalSamples) return t("已收到声音，等待播放输出");
  return t("声音已送往播放端");
});

const cableOutputs = computed(() => outputs.value.filter(o => /^CABLE Input(?:\s*\(|$)/i.test(o.name)));
const cablePresent = computed(() =>
  serviceInstalled.value ||
  cableOutputs.value.length > 0 ||
  inputs.value.some(i => /^CABLE (?:Output|Out)(?:\s*\(|$)/i.test(i.name)),
);

function readDevices(value: ActionValue, readGainRevision: number) {
  outputs.value = value.outputs;
  inputs.value = value.recordingInputs ?? [];
  serviceInstalled.value = !!value.cableInstalled;
  selected.value = value.selected;
  if (value.gainDb !== undefined && readGainRevision === gainRevision && !gainBusy.value) {
    savedGainDb.value = gainDb.value = value.gainDb;
  }
  verifiedInput.value = outputs.value.some(o => o.id === value.selected) ? value.verifiedInput?.name || "" : "";
  loaded.value = true;
}
async function saveGain() {
  if (gainBusy.value) return;
  gainBusy.value = true;
  gainRevision++;
  const requested = gainDb.value;
  message.value = "";
  try {
    const result = await invoke<{ gainDb?: number }>("microphone_action", { action: "gain", id: String(requested) });
    savedGainDb.value = gainDb.value = result.gainDb ?? requested;
  } catch (error) {
    gainDb.value = savedGainDb.value;
    message.value = errorText(error, t("操作失败"));
  } finally {
    gainRevision++;
    gainBusy.value = false;
  }
}

async function stopReceiving() {
  if (stopBusy.value) return;
  stopBusy.value = true;
  message.value = "";
  try {
    await invoke("microphone_action", { action: "stop" });
  } catch (error) {
    message.value = errorText(error, t("操作失败"));
  } finally {
    stopBusy.value = false;
  }
}

async function configure(id: string) {
  if (!id) {
    message.value = t("请选择虚拟线缆播放端；没有设备时，请先安装虚拟音频线缆。");
    return;
  }
  const result = await invoke<ActionValue>("microphone_action", { action: "setup", id });
  selected.value = result.selected;
  verifiedInput.value = result.target?.name || "";
}

// The device list can change while preferences are open.
async function setup(automatic = false) {
  if (busy.value || props.status?.active) return;
  const manual = selected.value;
  const readGainRevision = gainRevision;
  busy.value = true;
  message.value = "";
  try {
    readDevices(await invoke<ActionValue>("microphone_action", { action: "list" }), readGainRevision);
    const virtual = outputs.value.filter(o => o.virtualHint);
    const saved = virtual.find(o => o.id === selected.value);
    const candidate = cableOutputs.value.length === 1 ? cableOutputs.value[0]
      : saved || (virtual.length === 1 ? virtual[0] : undefined);
    verifiedInput.value = "";
    if (automatic && !candidate) {
      message.value = t("未找到唯一可用线路，请安装 VB-CABLE 或手动选择播放端。");
      return;
    }
    await configure((automatic ? candidate?.id : manual || candidate?.id) || "");
  } catch (error) {
    message.value = errorText(error, t("操作失败"));
  } finally {
    busy.value = false;
  }
}

async function action(actionName: string) {
  if (busy.value) return;
  const readGainRevision = gainRevision;
  busy.value = true;
  message.value = "";
  installing.value = actionName === "installCable";
  try {
    const value = await invoke<ActionValue>("microphone_action", {
      action: actionName,
      id: selected.value,
    });
    readDevices(value, readGainRevision);
    if (actionName === "installCable") {
      if (cableOutputs.value.length === 1) await configure(cableOutputs.value[0].id);
      else message.value = t("安装程序已关闭。请按官方提示重启 Windows，然后重新检测。");
    }
  } catch (error) {
    message.value = errorText(error, t("操作失败"));
  } finally {
    busy.value = false;
    installing.value = false;
  }
}
</script>

<template>
  <details
    class="settings microphone-settings"
    @toggle="($event.target as HTMLDetailsElement).open && action('list')"
  >
    <summary>
      <span class="section-title">
        <Mic aria-hidden="true" />
        <span class="audio-heading-copy">
          <span>{{ t('手机麦克风') }}</span>
          <span v-if="status?.active" class="audio-live" :title="status.deviceName">{{ status.receivedFrames ? t('已收到音频') : t('等待音频') }} · {{ status.deviceName }}</span>
        </span>
      </span>
      <ChevronDown aria-hidden="true" />
    </summary>
    <div class="details-body">
      <div class="mic-section">
        <p v-if="loaded" class="mic-setup-status" role="status">{{ cablePresent
          ? t('已检测到 VB-CABLE。')
          : t('未检测到 VB-CABLE。已有其他虚拟声卡也可以直接检测。') }}</p>
        <p v-if="loaded && cablePresent && !verifiedInput" class="hint">{{ cableOutputs.length
          ? t('尚未验证录音通路，点击「检测并配置麦克风」即可测试。')
          : t('驱动已存在，但标准播放端不可用。请检查 Windows 声音设置，或在安装后重启再检测。') }}<button v-if="!cableOutputs.length" type="button" class="mic-reinstall-link" :disabled="busy || status?.active" @click="action('installCable')">{{ t('点击重新安装') }}</button></p>
        <div class="diagnostic-actions mic-setup-actions">
          <button class="accent" :disabled="busy || !loaded || status?.active" @click="setup(true)">{{ t('检测并配置麦克风') }}</button>
          <button v-if="loaded && !cablePresent" :disabled="busy || status?.active" @click="action('installCable')">{{ t('安装 VB-CABLE') }}</button>
        </div>
        <p v-if="installing" role="status">{{ t('正在下载或等待官方安装程序，请在 Windows 提示中确认。') }}</p>
        <p v-else-if="loaded && !cablePresent" class="hint mic-license-note">{{ t('音频驱动 VB-CABLE 由 VB-Audio 提供') }}</p>
      </div>
      <div class="mic-section">
        <label class="audio-route">
          {{ t('音频送往') }}
          <select v-model="selected" :disabled="busy" :aria-label="t('麦克风输出设备')" @change="verifiedInput = ''">
            <option value="">{{ t('未启用') }}</option>
            <option v-for="output in outputs" :key="output.id" :value="output.id">{{ output.name }}</option>
          </select>
        </label>
        <div class="diagnostic-actions mic-device-actions">
          <button :disabled="busy || !loaded || status?.active" @click="setup()">{{ t('播放测试音并配置') }}</button>
          <button :disabled="busy || !loaded" @click="action('select')">{{ t('保存音频设备') }}</button>
          <button :disabled="busy" @click="action('list')">{{ t('刷新设备') }}</button>
        </div>
        <p v-if="verifiedInput" role="status">{{ t('录音端已收到测试音，请在收音软件中选择「{0}」。', verifiedInput) }}</p>
        <p v-if="loaded && !outputs.length" class="hint">{{ t('未发现可用播放设备，请安装并启用虚拟音频线缆后刷新。') }}</p>
        <p v-if="message || status?.error" class="hint" role="status">{{ displayMessage(message || status?.error) }}</p>
      </div>
      <div class="mic-section mic-controls">
        <label class="mic-gain-heading" for="microphone-gain">{{ t('麦克风增益') }}<output for="microphone-gain">+{{ gainDb }} dB</output></label>
        <input id="microphone-gain" v-model.number="gainDb" type="range" min="0" max="36" step="3" :disabled="!loaded || gainBusy" :aria-label="t('麦克风增益')" :aria-valuetext="`+${gainDb} dB`" aria-describedby="gain-help" @change="saveGain" />
        <p id="gain-help" class="hint" :class="{ 'gain-clipping': gainClipping }" role="status">{{ gainClipping ? t('增益过高，请调低。') : t('声音失真时请调低增益。') }}</p>
      </div>
      <div v-if="status?.active" class="audio-signal">
        <div class="mic-signal-heading"><p role="status">{{ signalState }}</p><button :disabled="stopBusy" @click="stopReceiving">{{ t('停止接收') }}</button></div>
        <div class="audio-levels">
          <label>{{ t('手机信号') }}<meter min="0" max="1" :value="status.inputLevel ?? 0" :aria-label="t('手机音频信号')" /></label>
          <label>{{ t('播放信号') }}<meter min="0" max="1" :value="status.outputLevel ?? 0" :aria-label="t('Windows 播放信号')" /></label>
        </div>
      </div>
    </div>
  </details>
</template>

<style scoped>
.mic-reinstall-link {
  display: inline;
  min-height: 0;
  margin-inline-start: 4px;
  padding: 0;
  border: 0;
  border-radius: 3px;
  background: transparent;
  box-shadow: none;
  color: var(--accent);
  font: inherit;
  line-height: inherit;
  text-decoration: none;
  white-space: nowrap;
  vertical-align: baseline;
}
.mic-reinstall-link:hover:not(:disabled) { background: transparent; color: var(--accent-hover); }
.mic-reinstall-link:active:not(:disabled) { background: transparent; color: var(--accent-active); }
</style>
