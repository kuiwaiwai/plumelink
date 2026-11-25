<script setup lang="ts">
import { computed, ref } from "vue";
import { ChevronDown, Activity } from "@lucide/vue";
import { displayMessage, t } from "../../../shared/i18n";
import { errorText } from "../../../shared/format";
import type { DesktopState } from "../../../shared/protocol";
import { desktop } from "../bridge";
import { version } from "../../../package.json";
import MicrophoneDiagnostics from "./MicrophoneDiagnostics.vue";

const props = defineProps<{
  state: DesktopState;
  url: string;
}>();

const busy = ref(false);
const diagnosticMessage = ref("");
const diagnosticResult = ref<{ pointer: { x: number; y: number }; volume: number; muted: boolean; altHeld: boolean; magnifier: boolean } | null>(null);
const diagnosticsPanel = ref<HTMLDetailsElement>();
const micDiag = ref<{ refresh?: () => void } | null>(null);
const networkName = () => props.state.addresses.find(address => address.address === props.state.selected)?.name || t('未选择');
const lanRunning = computed(() => props.state.running && (props.state.lanRunning ?? true));
const usbOnly = computed(() => props.state.running && !lanRunning.value);
const discoveryStatus = computed(() => !lanRunning.value ? t('未运行') : props.state.discovery.active ? t('已启动广播') : props.state.discovery.error ? t('广播启动失败') : t('未广播'));

function onDiagnosticsToggle(event: Event) {
  if ((event.target as HTMLDetailsElement).open) micDiag.value?.refresh?.();
}

async function copyDiagnostics() {
  if (busy.value) return;
  busy.value = true;
  const report = [
    t("羽连 v{0}", version),
    t("服务：{0}", props.state.running ? t("运行中") : t("已停止")),
    t("无线连接：{0}", lanRunning.value ? t("运行中") : t("未启用")),
    t("地址：{0}", lanRunning.value ? props.url || t("不可用") : t("未启用")),
    ...(usbOnly.value ? [t("连接方式：{0}", t("USB 有线连接"))] : []),
    t("网卡数：{0}", props.state.addresses.length),
    t("当前网卡：{0}", networkName()),
    t("配对码更新：{0}", props.state.pairingIntervalMinutes === 0 ? t('手动更新') : t('{0} 分钟', props.state.pairingIntervalMinutes)),
    t("绑定设备：{0}", props.state.pairedCount),
    t("在线设备：{0}", props.state.connectedCount),
    t("控制模式：{0}", props.state.controlMode === 'free' ? t('自由模式') : t('主控模式')),
    t("局域网发现：{0}", discoveryStatus.value),
    t("错误：{0}", props.state.error ? displayMessage(props.state.error) : t("无")),
    ...(lanRunning.value && props.state.discovery.error ? [t("自动发现提示：{0}", displayMessage(props.state.discovery.error))] : []),
    t("手机麦克风：{0}", props.state.microphone?.active ? t('收音中') : t('未在使用')),
  ].join("\n");
  try {
    await navigator.clipboard.writeText(report);
    diagnosticMessage.value = t("诊断信息已复制（不包含令牌与配对码）");
  } catch {
    diagnosticMessage.value = t("复制失败，请手动记录状态");
  } finally {
    busy.value = false;
  }
}

async function runDiagnostics() {
  if (busy.value) return;
  busy.value = true;
  diagnosticMessage.value = t("正在检查 Windows 输入与音量接口…");
  try {
    diagnosticResult.value = await desktop.diagnostic();
    diagnosticMessage.value = t("本机控制接口工作正常");
  } catch (error) {
    diagnosticResult.value = null;
    diagnosticMessage.value = t("自检失败：{0}", errorText(error, t("操作失败")));
  } finally {
    busy.value = false;
  }
}

async function openLogsDir() {
  if (busy.value) return;
  busy.value = true;
  try {
    const dir = await desktop.openLogs();
    diagnosticMessage.value = t("已打开日志目录：{0}", dir);
  } catch (error) {
    diagnosticMessage.value = t("打开日志目录失败：{0}", errorText(error, t("操作失败")));
  } finally {
    busy.value = false;
  }
}

function focusPanel() {
  const panel = diagnosticsPanel.value;
  if (!panel) return;
  panel.open = true;
  panel.querySelector("summary")?.focus({ preventScroll: true });
  panel.scrollIntoView({ block: "nearest" });
}

defineExpose({ focusPanel });
</script>

<template>
  <details class="help" open>
    <summary>{{ t('连接遇到问题') }}<ChevronDown aria-hidden="true" /></summary>
    <div class="details-body">
      <p>{{ t('手机请使用羽连 App 连接。无线连接时，请先启动电脑端的遥控服务；有线连接请在“偏好设置 → USB 有线连接”中选择手机。') }}</p>
      <h3 class="connection-help-heading">{{ t('USB 有线连接') }}</h3>
      <p>{{ t('USB 连接不需要同一 Wi-Fi。请使用支持数据传输的数据线，开启手机 USB 调试并允许这台电脑；首次连接仍需在羽连中确认电脑身份和配对码。') }}</p>
      <p>{{ t('有线连接中断时，重新插接数据线并确认手机授权；仍未恢复时，在“偏好设置 → USB 有线连接”点击“重新检测”，再连接手机。') }}</p>
      <h3 class="connection-help-heading">{{ t('无线连接') }}</h3>
      <p>{{ t('手机连上 Wi-Fi，电脑连同一个路由器；电脑用网线也可以。若手机在使用移动数据、访客 Wi-Fi 或公司网络，请先换到普通家庭 Wi-Fi 再试。') }}</p>
      <p>{{ t('找不到电脑时，点手机上的“重新搜索”，或扫描电脑二维码；也可以手动输入电脑显示的连接地址。') }}</p>
      <p>{{ t('扫码仍连不上时，核对电脑上的 6 位配对码；过期后用新码。首次连接出现电脑身份确认时，请核对显示的地址。') }}</p>
      <p>{{ t('如果 Windows 弹出网络访问提示，请允许羽连在“专用网络”中通信。曾拒绝过时，可在 Windows 防火墙设置中重新允许。') }}</p>
      <p>{{ t('电脑开着 VPN、同时连了多个网络，或换过 Wi-Fi 后，请到“偏好设置 → 连接设置”选择正在使用的网卡，再重新启动遥控。') }}</p>
      <p>{{ t('连接成功却不能操作时，检查设备管理中的遥控权限；电脑锁屏或弹出系统确认窗口时，需要在电脑上完成操作。') }}</p>
    </div>
  </details>
  <details ref="diagnosticsPanel" id="desktop-diagnostics" class="help" @toggle="onDiagnosticsToggle">
    <summary><span class="section-title"><Activity aria-hidden="true" />{{ t('连接诊断') }}</span><ChevronDown aria-hidden="true" /></summary>
    <div class="details-body diagnostics">
      <dl>
        <div><dt>{{ t('遥控服务') }}</dt><dd>{{ state.running ? t('运行中') : t('已停止') }}</dd></div>
        <div><dt>{{ t('无线连接') }}</dt><dd>{{ lanRunning ? t('运行中') : t('未启用') }}</dd></div>
        <div v-if="usbOnly"><dt>{{ t('连接方式') }}</dt><dd>{{ t('USB 有线连接') }}</dd></div>
        <div><dt>{{ t('连接地址') }}</dt><dd>{{ lanRunning ? url || t('不可用') : t('未启用') }}</dd></div>
        <div><dt>{{ t('当前网卡') }}</dt><dd>{{ networkName() }}</dd></div>
        <div><dt>{{ t('可用网卡') }}</dt><dd>{{ state.addresses.length }}</dd></div>
        <div><dt>{{ t('自动发现') }}</dt><dd>{{ discoveryStatus }}</dd></div>
        <div><dt>{{ t('配对码更新') }}</dt><dd>{{ state.pairingIntervalMinutes === 0 ? t('手动更新') : t('{0} 分钟', state.pairingIntervalMinutes) }}{{ state.pairingIntervalMinutes !== 0 && state.running ? t(' · 剩余 {0} 分钟', Math.ceil(state.codeRemaining / 60)) : '' }}</dd></div>
        <div><dt>{{ t('设备') }}</dt><dd>{{ state.connectedCount }} {{ t('在线 /') }} {{ state.pairedCount }} {{ t('已绑定') }}</dd></div>
        <div><dt>{{ t('控制模式') }}</dt><dd>{{ state.controlMode === 'free' ? t('自由模式') : t('主控模式') }}</dd></div>
        <div><dt>{{ t('手机麦克风') }}</dt><dd>{{ state.microphone?.active ? t('收音中') : t('未在使用') }}</dd></div>
        <div v-if="state.microphone?.active"><dt>{{ t('声音接收') }}</dt><dd>{{ state.microphone.receivedFrames }} {{ t('帧') }} · {{ state.microphone.renderedSignalSamples }} {{ t('个样本已送出') }}</dd></div>
        <div v-if="state.error"><dt>{{ t('服务提示') }}</dt><dd>{{ displayMessage(state.error) }}</dd></div>
      </dl>
      <p v-if="lanRunning && state.discovery.error" class="hint" role="status">{{ displayMessage(state.discovery.error) }}{{ t('。可先用二维码或地址连接。') }}</p>
      <div class="diagnostic-actions">
        <button :disabled="busy" @click="runDiagnostics">{{ t('运行本机自检') }}</button>
        <button :disabled="busy" @click="copyDiagnostics">{{ t('复制脱敏诊断信息') }}</button>
        <button :disabled="busy" @click="openLogsDir">{{ t('打开日志目录') }}</button>
      </div>
      <p v-if="diagnosticResult" class="hint">{{ t('鼠标位置') }} {{ diagnosticResult.pointer.x }}, {{ diagnosticResult.pointer.y }} {{ t('· 音量') }} {{ diagnosticResult.volume < 0 ? t('不可用') : `${Math.round(diagnosticResult.volume * 100)}%` }}{{ diagnosticResult.muted ? t('（静音）') : '' }} · Alt {{ diagnosticResult.altHeld ? t('按下') : t('已释放') }} {{ t('· 放大镜') }} {{ diagnosticResult.magnifier ? t('开启') : t('已关闭') }}</p>
      <p v-if="diagnosticMessage" class="hint" role="status">{{ displayMessage(diagnosticMessage) }}</p>
      <MicrophoneDiagnostics ref="micDiag" :status="state.microphone" />
    </div>
  </details>
</template>

<style scoped>
.connection-help-heading { margin: 18px 0 8px; font-size: 13px; font-weight: 600; color: var(--text); }
</style>
