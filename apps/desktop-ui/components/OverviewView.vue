<script setup lang="ts">
import { computed } from "vue";
import { Copy, ChevronDown, PanelBottomClose, Smartphone, ShieldCheck, Wifi, Monitor } from "@lucide/vue";
import { t } from "../../../shared/i18n";
import type { DesktopState } from "../../../shared/protocol";

const props = defineProps<{
  state: DesktopState;
  initializing: boolean;
  busy: boolean;
  qr: string;
  url: string;
  showPairing: boolean;
}>();

const emit = defineEmits<{
  "update:showPairing": [value: boolean];
  action: [name: string];
  navigate: [section: string];
  notify: [message: string];
}>();

const title = computed(() =>
  !props.state.running ? t("遥控已停止")
    : props.state.connected ? t("{0} 台设备在线", props.state.connectedCount)
    : props.state.trusted ? t("已绑定 {0} 台设备", props.state.pairedCount)
    : t("等待手机配对"),
);

const description = computed(() =>
  !props.state.running ? t("启动遥控后，手机才能连接。已保存的配对会保留。")
    : props.state.connected
      ? props.state.controlMode === "primary"
        ? t("主控模式已开启，只有指定设备可以操作。")
        : props.state.connectedCount > 1
          ? t("多台设备可同时操控{0}。", props.state.recentController ? t("，最近操作：{0}", props.state.recentController) : "")
          : t("设备已连接，可以开始遥控。")
    : props.state.trusted
      ? t("绑定记录已保留；也可以继续扫码添加新设备。")
      : t("手机与电脑连接同一路由器，使用羽连 App 扫码即可配对。"),
);

const codeStatus = computed(() =>
  props.state.pairingIntervalMinutes === 0
    ? t('手动更新')
    : props.state.codeRemaining > 0
    ? t("剩余 {0} 分钟 · 到期自动更新", Math.ceil(props.state.codeRemaining / 60))
    : t("正在更新配对码…"),
);

async function copyAddress() {
  try {
    await navigator.clipboard.writeText(props.url);
    emit("notify", t("地址已复制"));
  } catch {
    emit("notify", t("复制失败，请选中地址手动复制"));
  }
}
</script>

<template>
  <section id="desktop-overview" class="status hero-card" :class="{ connected: state.connected && state.running }" aria-live="polite" :aria-busy="initializing">
    <div class="status-heading"><span class="dot" :class="{ live: state.running && state.connected }"></span><h2>{{ initializing ? t('正在读取服务状态…') : title }}</h2></div>
    <p>{{ description }}</p>
    <button v-if="state.connected && state.running" class="accent background-button" :disabled="busy" @click="emit('action', 'hide')"><PanelBottomClose aria-hidden="true" />{{ t('后台运行') }}</button>
    <button v-if="!state.running" class="accent" :disabled="busy" @click="emit('action', 'start')">{{ t('启动遥控') }}</button>
  </section>
  <section class="overview" :aria-label="t('连接概览')">
    <div><Smartphone aria-hidden="true" /><span><b>{{ state.connectedCount }} <small>/ {{ state.pairedCount }}</small></b><small>{{ t('在线 / 已绑定') }}</small></span></div>
    <div><ShieldCheck aria-hidden="true" /><span><b>{{ state.running ? t('加密连接') : t('等待启动') }}</b><small>{{ t('仅授权设备可访问') }}</small></span></div>
    <div><Wifi aria-hidden="true" /><span><b>{{ state.discovery.active ? t('可自动发现') : state.running ? t('使用扫码连接') : t('发现已暂停') }}</b><small>{{ t('同一局域网') }}</small></span></div>
  </section>
  <div class="connect-layout">
    <div class="connect-panel">
      <div class="workspace-heading"><h2>{{ t('连接这台电脑') }}</h2><span>{{ t('Android · 局域网连接') }}</span></div>
      <button v-if="state.running && state.trusted" class="pair-toggle" :aria-expanded="showPairing" @click="emit('update:showPairing', !showPairing)">{{ t('扫码与配对信息') }}<ChevronDown aria-hidden="true" :class="{ rotated: showPairing }" /></button>
      <section v-if="state.running && (!state.trusted || showPairing)" class="connection-grid">
        <div class="qr-card"><div class="qr-frame"><img v-if="qr" :src="qr" :alt="t('使用羽连 App 扫码配对')" /><span v-else role="status">{{ t('正在生成二维码…') }}</span></div><p>{{ t('使用 App 扫码配对') }}</p></div>
        <div class="pair-details"><h3>{{ t('6 位配对码') }}</h3><div class="pair-code" :class="{ expired: state.pairingIntervalMinutes !== 0 && state.codeRemaining <= 0 }">{{ state.code }}</div><p class="code-status">{{ codeStatus }}</p><button class="text-button" :disabled="busy" @click="emit('action', 'pair')">{{ t('刷新配对码') }}</button></div>
      </section>
      <section class="address-row connection-address"><div><p class="hint">{{ t('当前连接地址') }}{{ state.running ? '' : t('（服务已停止）') }}</p><code>{{ url || t('暂无可用局域网地址') }}</code></div><button :disabled="!url" @click="copyAddress"><Copy aria-hidden="true" />{{ t('复制地址') }}</button></section>
    </div>
    <aside class="connect-guide">
      <span class="guide-icon"><Monitor aria-hidden="true" /></span>
      <h2>{{ t('从手机开始连接') }}</h2>
      <p>{{ t('手机与电脑连接同一路由器，即可开始使用。') }}</p>
      <ol>
        <li><b>{{ t('打开羽连 App') }}</b><span>{{ t('在手机端选择添加电脑') }}</span></li>
        <li><b>{{ t('扫描电脑二维码') }}</b><span>{{ t('核对地址并确认电脑证书') }}</span></li>
        <li><b>{{ t('自动完成配对') }}</b><span>{{ t('完成绑定，下次自动连接') }}</span></li>
      </ol>
      <button @click="emit('navigate', 'devices')"><Monitor aria-hidden="true" />{{ t('管理我的设备') }}</button>
      <button class="text-button" @click="emit('navigate', 'diagnostics')">{{ t('遇到连接问题？') }}</button>
    </aside>
  </div>
</template>
