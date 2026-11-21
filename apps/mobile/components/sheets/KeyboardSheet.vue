<script setup lang="ts">
import { ref } from "vue";
import { Send } from "@lucide/vue";
import { displayMessage, t } from "../../../../shared/i18n";
import type { Command } from "../../../../shared/protocol";

const props = defineProps<{
  canInput: boolean;
  canText: boolean;
  online: boolean;
  sending: boolean;
  connectionLabel: string;
  textFeedback: string;
}>();

const input = defineModel<string>("input", { required: true });
const composing = ref(false);

const emit = defineEmits<{
  command: [command: Command];
  submit: [];
}>();

function syncInput(event: Event) {
  if (!("isComposing" in event && (event as InputEvent).isComposing)) composing.value = false;
}

function onTextEnter(event: KeyboardEvent) {
  if (event.isComposing || composing.value || event.keyCode === 229) return;
  event.preventDefault();
  emit("submit");
}

function resetComposing() {
  composing.value = false;
}

defineExpose({ resetComposing });
</script>

<template>
  <form class="text-panel" @submit.prevent="emit('submit')">
    <div class="keyboard-shortcuts">
      <button v-for="item in ([['escape','Esc'],['tab','Tab'],['backspace',t('退格')],['delete','Del'],['enter',t('回车')],['home','Home'],['end','End'],['pageup','PgUp'],['pagedown','PgDn'],['f5','F5'],['f11','F11']] as const)" :key="item[0]" type="button" :disabled="!canInput" @click="emit('command', { type: 'key', key: item[0] })">{{ item[1] }}</button>
    </div>
    <div class="keyboard-combos" :aria-label="t('快捷组合键')">
      <button v-for="combo in ([['a',t('全选')],['c',t('复制')],['v',t('粘贴')],['x',t('剪切')],['z',t('撤销')],['f',t('查找')],['w',t('关闭页')]] as const)" :key="combo[0]" type="button" :disabled="!canInput" @click="emit('command', { type: 'combo', modifiers: ['ctrl'], key: combo[0] })">{{ combo[1] }}</button>
    </div>
    <label for="remote-text">{{ t('输入到电脑当前光标位置') }}</label>
    <textarea id="remote-text" v-model="input" maxlength="1000" rows="3" enterkeyhint="send" data-initial-focus
      :placeholder="t('输入文字，发送到电脑…')" @input="syncInput" @compositionstart="composing = true" @compositionend="composing = false" @blur="composing = false" @keydown.enter.exact="onTextEnter"></textarea>
    <div class="text-meta"><span>{{ t('不读取或修改剪贴板') }}</span><span>{{ input.length }} / 1000</span></div>
    <p v-if="!online" class="panel-feedback" role="status">{{ connectionLabel }}{{ t('，草稿已保留。') }}</p>
    <p v-if="textFeedback" class="panel-feedback" role="status">{{ displayMessage(textFeedback) }}</p>
    <button class="accent send-button" type="submit" :disabled="!canText || sending || !input || composing"><Send aria-hidden="true" />{{ sending ? t('发送中…') : t('发送文字') }}</button>
  </form>
</template>
