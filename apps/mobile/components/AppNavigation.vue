<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../shared/i18n";

import { MousePointer2, Files, Settings2 } from "@lucide/vue";
export type Destination = "remote" | "files" | "settings" | "keyboard" | "windows";
defineProps<{ active: Destination; transferring?: boolean }>();
defineEmits<{ navigate: [value: Destination] }>();
const items = computed(() => ([{ value: "remote", label: t('遥控'), icon: MousePointer2 }, { value: "files", label: t('文件'), icon: Files }, { value: "settings", label: t('设置'), icon: Settings2 }] as const));
</script>
<template>
  <nav class="app-navigation" :aria-label="t('主导航')">
    <button v-for="item in items" :key="item.value" :aria-current="active === item.value ? 'page' : undefined" @click="$emit('navigate', item.value)">
      <span class="nav-icon"><component :is="item.icon" aria-hidden="true" /><i v-if="item.value === 'files' && transferring" :aria-label="t('正在传输')"></i></span>
      <span>{{ item.label }}</span>
    </button>
  </nav>
</template>
