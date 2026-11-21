<script setup lang="ts">
import { Power } from "@lucide/vue";
import { t } from "../../../../shared/i18n";

defineProps<{
  canPower: boolean;
  shutdownPending: boolean;
  shutdownSent: boolean;
}>();

const emit = defineEmits<{
  shutdown: [];
  close: [];
}>();
</script>

<template>
  <section class="window-panel">
    <p>{{ t('让 Windows 正常关闭应用（包括羽连）并关机。未保存的文件可能阻止关机，请先保存重要内容。') }}</p>
    <p>{{ t('不会强制结束应用。如果取消了 Windows 关机，羽连仍可继续使用。') }}</p>
    <button class="danger-button" :disabled="!canPower || shutdownPending || shutdownSent" @click="emit('shutdown')"><Power aria-hidden="true" />{{ shutdownSent ? t('已提交，请查看电脑') : t('验证身份并关机') }}</button>
    <button @click="emit('close')">{{ t('返回遥控') }}</button>
  </section>
</template>
