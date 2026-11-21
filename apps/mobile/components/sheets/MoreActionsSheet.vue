<script setup lang="ts">
import { computed } from "vue";
import { Monitor, Delete, ArrowUp, ArrowDown, PanelsTopLeft } from "@lucide/vue";
import { t } from "../../../../shared/i18n";
import type { Command } from "../../../../shared/protocol";
import { shortcutIcons } from "../../shortcut-icons";

defineProps<{
  canInput: boolean;
  shortcutA: string;
  shortcutB: string;
  shortcutC: string;
  shortcutIconA: string;
  shortcutIconB: string;
  shortcutIconC: string;
}>();

const emit = defineEmits<{
  command: [command: Command];
  shortcut: [value: string];
  windows: [];
}>();

const shortcutLabels = computed<Record<string, string>>(() => ({
  mute: t("静音"),
  magnifier: t("鼠标放大镜"),
  taskview: t("任务视图"),
  notifications: t("通知中心"),
  previousapp: t("上一个应用"),
  nextapp: t("下一个应用"),
  virtualleft: t("上一个桌面"),
  virtualright: t("下一个桌面"),
}));
</script>

<template>
  <div class="more-grid">
    <button :disabled="!canInput" @click="emit('command', { type: 'key', key: 'escape' })"><kbd class="esc-key" aria-hidden="true">esc</kbd><span>Esc</span></button>
    <button :disabled="!canInput" @click="emit('command', { type: 'desktop' })"><Monitor aria-hidden="true" /><span>{{ t('显示桌面') }}</span></button>
    <button :disabled="!canInput" :aria-label="t('删除（Backspace）')" @click="emit('command', { type: 'key', key: 'backspace' })"><Delete aria-hidden="true" /><span>{{ t('删除') }}</span></button>
    <button :disabled="!canInput" @click="emit('command', { type: 'scroll', dy: -120 })"><ArrowUp aria-hidden="true" /><span>{{ t('向上滚动') }}</span></button>
    <button :disabled="!canInput" @click="emit('command', { type: 'scroll', dy: 120 })"><ArrowDown aria-hidden="true" /><span>{{ t('向下滚动') }}</span></button>
    <button :disabled="!canInput" @click="emit('windows')"><PanelsTopLeft aria-hidden="true" /><span>{{ t('切换窗口') }}</span></button>
    <button :disabled="!canInput" @click="emit('shortcut', shortcutA)"><component :is="shortcutIcons[shortcutIconA]" aria-hidden="true" /><span>{{ shortcutLabels[shortcutA] }}</span></button>
    <button :disabled="!canInput" @click="emit('shortcut', shortcutB)"><component :is="shortcutIcons[shortcutIconB]" aria-hidden="true" /><span>{{ shortcutLabels[shortcutB] }}</span></button>
    <button :disabled="!canInput" @click="emit('shortcut', shortcutC)"><component :is="shortcutIcons[shortcutIconC]" aria-hidden="true" /><span>{{ shortcutLabels[shortcutC] }}</span></button>
  </div>
</template>
