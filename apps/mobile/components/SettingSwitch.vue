<script setup lang="ts">
import { Check } from "@lucide/vue";
defineProps<{ modelValue: boolean; label: string; description?: string }>();
defineEmits<{ 'update:modelValue': [value: boolean] }>();
</script>
<template>
  <button class="setting-row switch-row" type="button" role="switch" :aria-checked="modelValue" :aria-label="label" @click="$emit('update:modelValue', !modelValue)">
    <span class="row-copy"><strong>{{ label }}</strong><small v-if="description">{{ description }}</small></span>
    <span class="switch-track" :class="{ checked: modelValue }" aria-hidden="true"><span><Check v-if="modelValue" /></span></span>
  </button>
</template>
