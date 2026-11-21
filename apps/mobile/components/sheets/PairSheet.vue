<script setup lang="ts">
import { Monitor, ScanSearch } from "@lucide/vue";
import { displayMessage, t } from "../../../../shared/i18n";

defineProps<{
  computerName: string;
  connectionAddress: string;
  hasScanner: boolean;
  pairing: boolean;
  status: string;
  message: string;
}>();

const code = defineModel<string>("code", { required: true });

const emit = defineEmits<{
  scan: [];
  submit: [];
}>();

function onInput(event: Event) {
  const input = event.target as HTMLInputElement;
  const cleaned = input.value.replace(/[^0-9]/g, "").slice(0, 6);
  input.value = cleaned;
  code.value = cleaned;
}
</script>

<template>
  <form class="pair-panel" @submit.prevent="emit('submit')">
    <div class="pair-computer"><Monitor aria-hidden="true" /><span><strong>{{ computerName }}</strong><small>{{ connectionAddress }}</small></span></div>
    <button v-if="hasScanner" class="tonal-button" type="button" :disabled="pairing" @click="emit('scan')"><ScanSearch aria-hidden="true" />{{ t('扫描电脑二维码') }}</button>
    <label for="pair-code">{{ t('输入电脑上显示的 6 位配对码') }}</label>
    <input id="pair-code" :value="code" @input="onInput" type="text" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" autocomplete="off" :placeholder="t('6 位配对码')" required data-initial-focus />
    <p class="pair-status" role="status" aria-live="polite">{{ displayMessage(message) }}</p>
    <button class="accent" :disabled="pairing || !/^[0-9]{6}$/.test(code) || status === 'connecting'" type="submit">{{ pairing ? t('正在配对…') : t('配对连接') }}</button>
  </form>
</template>
