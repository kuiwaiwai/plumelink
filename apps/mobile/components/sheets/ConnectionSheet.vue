<script setup lang="ts">
import { Monitor } from "@lucide/vue";
import { displayMessage, t } from "../../../../shared/i18n";

defineProps<{
  computerName: string;
  connectionAddress: string;
  message: string;
  qualityLabel: string;
  latency: number | null;
  hasControl: boolean;
  primaryName: string;
  adminGranted: boolean;
  online: boolean;
  status: string;
  hasNative: boolean;
  transportStats: { intervalMs: number; queueMs: number };
  permissions: { input: boolean; text: boolean; files: boolean };
}>();

const emit = defineEmits<{
  openPair: [];
  resume: [];
  changeComputer: [];
  forget: [];
}>();
</script>

<template>
  <section class="connection-panel">
    <div class="connection-hero"><Monitor aria-hidden="true" /><h3>{{ computerName }}</h3><p role="status">{{ displayMessage(message) }}</p><small class="address">{{ connectionAddress }}</small></div>
    <div class="settings-list">
      <div class="setting-row"><span class="row-copy"><strong>{{ t('网络质量') }}</strong><small>{{ qualityLabel }}</small></span><span>{{ latency === null ? '—' : `${latency} ms` }}</span></div>
      <div class="setting-row"><span class="row-copy"><strong>{{ t('控制权限') }}</strong><small>{{ hasControl ? t('可以控制') : t('当前由{0}主控', primaryName || t('另一台设备')) }}{{ adminGranted ? t(' · 管理员控制已开启') : '' }}</small></span></div>
    </div>
    <button v-if="!online" class="accent" @click="status === 'unpaired' ? emit('openPair') : emit('resume')">{{ status === 'unpaired' ? t('输入配对码') : t('重新连接') }}</button>
    <button v-if="hasNative" class="tonal-button" @click="emit('changeComputer')">{{ t('管理与切换电脑') }}</button>
    <button v-if="status !== 'unpaired'" class="text-button danger-text" @click="emit('forget')">{{ t('忘记此电脑') }}</button>
    <details class="connection-details">
      <summary>{{ t('连接详情') }}</summary>
      <p>{{ t('QFastTransmission · 发送间隔') }} {{ transportStats.intervalMs }} {{ t('ms · 电脑排队') }} {{ transportStats.queueMs }} ms</p>
      <p>{{ t('权限：遥控') }} {{ permissions.input ? t('允许') : t('已关闭') }} {{ t('· 文字') }} {{ permissions.text ? t('允许') : t('已关闭') }} {{ t('· 文件') }} {{ permissions.files ? t('允许') : t('已关闭') }}</p>
    </details>
  </section>
</template>
