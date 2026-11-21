<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { FolderOpen, FileUp, X } from "@lucide/vue";
import { t } from "../../../shared/i18n";
import { formatBytes } from "../../../shared/format";
import type { DesktopState } from "../../../shared/protocol";

const props = defineProps<{
  state: DesktopState;
  busy: boolean;
}>();

const emit = defineEmits<{
  transfer: [name: string, id?: number, ids?: number[]];
}>();

const selectedFiles = ref<number[]>([]);
const allFilesSelected = computed(() =>
  props.state.sharedFiles.length > 0 && selectedFiles.value.length === props.state.sharedFiles.length,
);

watch(
  () => props.state.sharedFiles,
  files => {
    const available = new Set(files.map(file => file.id));
    selectedFiles.value = selectedFiles.value.filter(id => available.has(id));
  },
);

function selectAllFiles(checked: boolean) {
  selectedFiles.value = checked ? props.state.sharedFiles.map(file => file.id) : [];
}

function fileKind(name: string) {
  const extension = name.includes(".") ? name.split(".").pop() : "";
  return extension && /^[a-z0-9]{1,6}$/i.test(extension) ? extension.toUpperCase() : t("文件");
}
</script>

<template>
  <section id="desktop-transfer" class="workspace-card" :aria-busy="busy">
    <div class="workspace-heading"><h2>{{ t('手机与电脑互传') }}</h2></div>
    <div class="details-body file-transfer">
      <div class="transfer-intro">
        <span class="transfer-symbol"><FileUp aria-hidden="true" /></span>
        <div><h3>{{ t('文件，即选即分享') }}</h3><p>{{ t('单文件最大 2 GB。') }}</p></div>
        <button class="accent" :disabled="busy" @click="emit('transfer', 'pickfile')">{{ t('选择文件') }}</button>
      </div>
      <div class="transfer-setting"><span><b>{{ t('Windows 接收目录') }}</b><small>{{ state.receivePath }}</small></span><button :disabled="busy" @click="emit('transfer', 'pickreceive')"><FolderOpen aria-hidden="true" />{{ t('选择目录') }}</button></div>
      <section class="shared-file-panel" :aria-label="t('共享文件管理')">
        <div class="file-list-toolbar">
          <label><input type="checkbox" :checked="allFilesSelected" :indeterminate="selectedFiles.length > 0 && !allFilesSelected" :disabled="busy || !state.sharedFiles.length" @change="selectAllFiles(($event.target as HTMLInputElement).checked)" />{{ t('全选') }} <span>{{ state.sharedFiles.length }} {{ t('个文件') }}</span></label>
          <div>
            <button :disabled="busy || !selectedFiles.length" @click="emit('transfer', 'removemany', undefined, [...selectedFiles])">{{ t('取消所选共享') }}<span class="file-selection-count">（{{ selectedFiles.length }}）</span></button>
            <button :disabled="busy || !state.sharedFiles.length" @click="emit('transfer', 'removemany', undefined, state.sharedFiles.map(file => file.id))">{{ t('全部取消共享') }}</button>
          </div>
        </div>
        <div class="shared-file-viewport" :tabindex="state.sharedFiles.length ? 0 : undefined" :aria-label="t('共享文件列表')">
          <ul class="shared-files">
            <li v-for="file in state.sharedFiles" :key="file.id" :class="{ selected: selectedFiles.includes(file.id) }">
              <label><input v-model="selectedFiles" type="checkbox" :value="file.id" :disabled="busy" :aria-label="t('选择 {0}', file.name)" /><span class="shared-file-copy"><b :title="file.name">{{ file.name }}</b><span class="shared-file-meta"><span class="shared-file-kind">{{ fileKind(file.name) }}</span><small>{{ formatBytes(file.size) }}</small></span></span></label>
              <button :aria-label="t('取消共享 {0}', file.name)" :title="t('取消共享，保留原文件')" :disabled="busy" @click="emit('transfer', 'remove', file.id)"><X aria-hidden="true" /></button>
            </li>
          </ul>
          <p v-if="!state.sharedFiles.length" class="empty-files">{{ t('暂无共享文件，选择后会显示在这里。') }}</p>
        </div>
      </section>
    </div>
  </section>
</template>
