<script setup lang="ts">
import { ChevronLeft, ChevronRight } from "@lucide/vue";
import { t } from "../../../../shared/i18n";

defineProps<{
  canInput: boolean;
  hint: string;
}>();

const emit = defineEmits<{
  cycle: [action: "previous" | "next"];
  finish: [action: "cancel" | "confirm"];
}>();
</script>

<template>
  <section class="window-panel">
    <p role="status">{{ hint }}</p>
    <div class="window-cycle">
      <button :disabled="!canInput" @click="emit('cycle', 'previous')"><ChevronLeft aria-hidden="true" />{{ t('上一个') }}</button>
      <button :disabled="!canInput" @click="emit('cycle', 'next')">{{ t('下一个') }}<ChevronRight aria-hidden="true" /></button>
    </div>
    <div class="window-cycle">
      <button :disabled="!canInput" @click="emit('finish', 'cancel')">{{ t('取消选择') }}</button>
      <button class="accent" :disabled="!canInput" @click="emit('finish', 'confirm')">{{ t('确认窗口') }}</button>
    </div>
    <small>{{ t('选择时保持 Alt；停止操作 3 秒自动释放。') }}</small>
  </section>
</template>
