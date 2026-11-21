<script setup lang="ts">
import { ref, type Component } from "vue";
import { Check, ChevronRight } from "@lucide/vue";
import BottomSheet from "./BottomSheet.vue";
type Option = { value: string; label: string; description?: string; icon?: Component };
const props = defineProps<{ id: string; label: string; modelValue: string; options: Option[] }>();
const emit = defineEmits<{ 'update:modelValue': [value: string] }>();
const expanded = ref(false);
function choose(value: string) { emit('update:modelValue', value); expanded.value = false; }
function navigate(event: KeyboardEvent) {
  if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
  const buttons = Array.from((event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("[role=radio]"));
  const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
  if (current < 0) return;
  event.preventDefault();
  const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : (current + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
  emit('update:modelValue', props.options[next].value); buttons[next]?.focus();
}
</script>
<template>
  <div class="setting-choice">
    <button :id="id" class="setting-row" type="button" :aria-label="label" aria-haspopup="dialog" :aria-expanded="expanded" @click="expanded = true">
      <span class="row-copy"><strong>{{ label }}</strong></span><span class="row-value"><component :is="options.find(o => o.value === modelValue)?.icon" v-if="options.find(o => o.value === modelValue)?.icon" aria-hidden="true" />{{ options.find(o => o.value === modelValue)?.label }}</span><ChevronRight aria-hidden="true" />
    </button>
    <BottomSheet :open="expanded" :title="label" @close="expanded = false">
      <div class="choice-options" role="radiogroup" :aria-label="label" @keydown="navigate"><button v-for="option in options" :key="option.value" class="choice-option" role="radio" :data-initial-focus="option.value === modelValue ? '' : undefined" :tabindex="option.value === modelValue ? 0 : -1" :aria-checked="option.value === modelValue" @click="choose(option.value)"><component :is="option.icon" v-if="option.icon" class="choice-option-icon" aria-hidden="true" /><span class="row-copy"><strong>{{ option.label }}</strong><small v-if="option.description">{{ option.description }}</small></span><span class="radio-mark" aria-hidden="true"><Check v-if="option.value === modelValue" /></span></button></div>
    </BottomSheet>
  </div>
</template>
