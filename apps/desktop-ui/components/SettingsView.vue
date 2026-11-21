<script setup lang="ts">
import { ChevronDown, Wifi, SlidersHorizontal, Palette, LogIn, ShieldCheck } from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { errorText } from "../../../shared/format";
import policy from "../../../PRIVACY.md?raw";
import { displayMessage, locale, t, languageOptions, languagePreference } from "../../../shared/i18n";
import { desktopTheme } from "../theme";
import { DEFAULT_SERVICE_PORT } from "../bridge";
import type { DesktopState } from "../../../shared/protocol";
import MicrophoneSettings from "./MicrophoneSettings.vue";

const headings = { "zh-CN": "简体中文", "zh-TW": "繁體中文", en: "English" };
const policyText = computed(() => policy.replace(/\r/g, "").split(/^## /m)
  .find(section => section.startsWith(headings[locale.value] + "\n"))?.split(/\n\s*\n/).slice(1).map(p => p.trim()).filter(Boolean));

const props = defineProps<{
  state: DesktopState;
  busy: boolean;
  savingHaloSize: boolean;
  setPort: (port: number) => Promise<void>;
}>();

const draftPort = ref(String(props.state.port));
const portError = ref("");
watch(() => props.state.port, port => { draftPort.value = String(port); });
watch(draftPort, () => { portError.value = ""; });
const portNumber = computed(() => /^\d{1,5}$/.test(draftPort.value) ? Number(draftPort.value) : NaN);
const validPort = computed(() => portNumber.value >= 1 && portNumber.value <= 65535);
const portChanged = computed(() => portNumber.value !== props.state.port);
async function applyPort() {
  if (props.busy || !validPort.value || !portChanged.value) return;
  portError.value = "";
  try {
    await props.setPort(portNumber.value);
  } catch (error) {
    portError.value = errorText(error, t("操作失败"));
  }
}

const selected = defineModel<string>("selected", { required: true });
const haloSize = defineModel<DesktopState["halo"]["size"]>("haloSize", { required: true });

const emit = defineEmits<{
  action: [name: string];
  saveHaloSize: [];
}>();
</script>

<template>
  <details class="settings" open>
    <summary><span class="section-title"><Palette aria-hidden="true" />{{ t('界面') }}</span><ChevronDown aria-hidden="true" /></summary>
    <div class="details-body preference-list">
      <div class="preference-row">
        <label for="desktop-language">{{ t('语言') }}</label>
        <select id="desktop-language" v-model="languagePreference"><option v-for="option in languageOptions" :key="option.value" :value="option.value">{{ option.label }}</option></select>
      </div>
      <div class="preference-row">
        <label for="desktop-theme">{{ t('外观') }}</label>
        <select id="desktop-theme" v-model="desktopTheme"><option value="system">{{ t('跟随系统') }}</option><option value="light">{{ t('浅色') }}</option><option value="dark">{{ t('深色') }}</option></select>
      </div>
    </div>
  </details>
  <details id="desktop-settings" class="settings" open>
    <summary><span class="section-title"><Wifi aria-hidden="true" />{{ t('连接设置') }}</span><ChevronDown aria-hidden="true" /></summary>
    <div class="details-body preference-list">
      <div class="preference-row network-row">
        <label for="network">{{ t('局域网网卡') }}</label>
        <select id="network" v-model="selected" :disabled="state.running || busy" :aria-describedby="state.running ? 'network-help' : undefined">
          <option value="" disabled>{{ t('请选择局域网网卡') }}</option>
          <option v-for="item in state.addresses" :key="item.address" :value="item.address">{{ item.name }} · {{ item.address }}{{ item.virtual ? t('（虚拟 / VPN）') : '' }}</option>
        </select>
        <p v-if="state.running" id="network-help" class="hint">{{ t('更换网卡请先停止遥控，再选择并启动。') }}</p>
      </div>
      <form class="preference-row port-row" @submit.prevent="applyPort">
        <label for="service-port">{{ t('服务端口') }}</label>
        <div class="port-controls">
          <input id="service-port" v-model="draftPort" type="text" inputmode="numeric" autocomplete="off" spellcheck="false"
            :disabled="busy" :aria-invalid="!validPort" :aria-describedby="portChanged ? 'port-feedback' : undefined" />
          <button type="submit" :disabled="busy || !validPort || !portChanged">{{ t('应用') }}</button>
        </div>
        <p v-if="portChanged" id="port-feedback" class="hint" role="status">
          {{ !validPort ? t('请输入 1 到 65535 之间的整数。') : portError ? displayMessage(portError) : state.running ? t('应用后，手机需要重新连接新端口。') : t('下次启动服务时使用此端口。') }}
          {{ t('当前端口：{0}', state.port) }}
        </p>
        <button v-if="state.port !== DEFAULT_SERVICE_PORT && portNumber !== DEFAULT_SERVICE_PORT" type="button" class="text-button port-default" :disabled="busy" @click="draftPort = String(DEFAULT_SERVICE_PORT)">{{ t('恢复默认端口（{0}）', DEFAULT_SERVICE_PORT) }}</button>
      </form>
      <div class="preference-row">
        <label for="pairing-interval">{{ t('配对码更新间隔') }}</label>
        <select id="pairing-interval" :value="state.pairingIntervalMinutes" :disabled="busy" @change="emit('action', `pairinginterval${($event.target as HTMLSelectElement).value}`)">
          <option v-for="minutes in [5, 10, 15, 30]" :key="minutes" :value="minutes">{{ t('{0} 分钟', minutes) }}</option>
          <option :value="0">{{ t('不更新') }}</option>
        </select>
      </div>
    </div>
  </details>
  <details class="settings" open>
    <summary><span class="section-title"><SlidersHorizontal aria-hidden="true" />{{ t('遥控设置') }}</span><ChevronDown aria-hidden="true" /></summary>
    <div class="details-body preference-list">
      <label class="startup-setting preference-row"><span>{{ t('鼠标定位光环') }}</span><input type="checkbox" :checked="state.halo.enabled" :disabled="busy" @change="emit('action', state.halo.enabled ? 'halooff' : 'haloon')" /></label>
      <div class="preference-row">
        <label for="halo-size">{{ t('光环大小') }}</label>
        <select id="halo-size" v-model="haloSize" :disabled="busy" :aria-busy="savingHaloSize" @change="emit('saveHaloSize')">
          <option value="small">{{ t('小') }}</option>
          <option value="medium">{{ t('中（默认）') }}</option>
          <option value="large">{{ t('大') }}</option>
        </select>
      </div>
    </div>
  </details>
  <MicrophoneSettings :status="state.microphone" />
  <label class="startup-setting"><span class="section-title"><LogIn aria-hidden="true" />{{ t('登录 Windows 后自动启动') }}</span><input type="checkbox" :checked="state.autostart" :disabled="busy" @change="emit('action', state.autostart ? 'autostartoff' : 'autostarton')" /></label>
  <details class="settings">
    <summary><span class="section-title"><ShieldCheck aria-hidden="true" />{{ t('隐私政策') }}</span><ChevronDown aria-hidden="true" /></summary>
    <article class="details-body privacy-copy"><template v-for="paragraph in policyText" :key="paragraph"><h3 v-if="paragraph.startsWith('### ')">{{ paragraph.slice(4) }}</h3><p v-else>{{ paragraph }}</p></template></article>
  </details>
</template>

<style scoped>
.port-controls { display: flex; align-items: center; gap: 8px; }
.port-controls input { width: 0; min-width: 0; flex: 1; min-height: 38px; padding: 4px 10px; font: inherit; }
.port-controls button { flex-shrink: 0; }
.port-row .hint, .port-default { grid-column: 2; }
.port-default { justify-self: start; margin: 0; }
@container preferences (max-width: 540px) {
  .port-row .hint, .port-default { grid-column: 1 / -1; }
}
.privacy-copy { display: grid; gap: 6px; line-height: 1.7; overflow-wrap: anywhere; }
.privacy-copy h3, .privacy-copy p { margin: 0; }
.privacy-copy h3 { color: var(--text); margin-top: 10px; font-size: 14px; }
.privacy-copy p { max-width: 76ch; }
</style>
