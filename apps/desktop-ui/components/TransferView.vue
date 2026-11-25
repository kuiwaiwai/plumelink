<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { FileUp, Files, Search, SearchX, X } from "@lucide/vue";
import { browseFiles, type FileOrder } from "../../../shared/file-list";
import { t, locale } from "../../../shared/i18n";
import { formatBytes } from "../../../shared/format";
import type { DesktopState } from "../../../shared/protocol";
import type { ReceiveAction } from "../../../shared/protocol";
import ReceivedFiles from "./ReceivedFiles.vue";

const props = defineProps<{
  state: DesktopState;
  busy: boolean;
  receiveAction: (action: ReceiveAction, taskId?: string) => Promise<void>;
}>();

const emit = defineEmits<{
  transfer: [name: string, id?: number, ids?: number[]];
}>();

const fileQuery = ref("");
const fileOrder = ref<FileOrder>("default");
const visibleFiles = computed(() => browseFiles(props.state.sharedFiles, fileQuery.value, fileOrder.value, locale.value));
const selectedFiles = ref<number[]>([]);
watch(fileQuery, () => { selectedFiles.value = []; });
const allFilesSelected = computed(() =>
  visibleFiles.value.length > 0 && visibleFiles.value.every(file => selectedFiles.value.includes(file.id)),
);

watch(
  () => visibleFiles.value,
  files => {
    const available = new Set(files.map(file => file.id));
    selectedFiles.value = selectedFiles.value.filter(id => available.has(id));
  },
);

function selectAllFiles(checked: boolean) {
  selectedFiles.value = checked ? visibleFiles.value.map(file => file.id) : [];
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
      <ReceivedFiles :files="state.receivedFiles ?? []" :directory="state.receivePath" :busy="busy" :action="receiveAction" @pick-directory="emit('transfer', 'pickreceive')" />
      <section class="shared-file-panel" :class="{ 'is-empty': !state.sharedFiles.length }" :aria-label="t('共享文件管理')">
        <header class="shared-file-heading">
          <h3>{{ t('共享文件列表') }}<span v-if="!state.sharedFiles.length" class="shared-file-total">0 {{ t('个文件') }}</span></h3>
        <div v-if="state.sharedFiles.length || fileQuery" class="file-browse-tools">
          <div class="file-search"><Search aria-hidden="true" /><input v-model="fileQuery" type="search" maxlength="120" :placeholder="t('搜索文件名')" :aria-label="t('搜索文件名')" /></div>
          <select v-model="fileOrder" :disabled="!state.sharedFiles.length" :aria-label="t('排序方式')"><option value="default">{{ t('默认顺序') }}</option><option value="name">{{ t('名称 A–Z') }}</option><option value="largest">{{ t('大小 从大到小') }}</option></select>
        </div>
        </header>
        <div v-if="state.sharedFiles.length" class="file-list-toolbar">
          <label><input type="checkbox" :checked="allFilesSelected" :indeterminate="selectedFiles.length > 0 && !allFilesSelected" :disabled="busy || !visibleFiles.length" @change="selectAllFiles(($event.target as HTMLInputElement).checked)" />{{ t('全选') }} <span>{{ fileQuery ? visibleFiles.length + ' / ' : '' }}{{ state.sharedFiles.length }} {{ t('个文件') }}</span></label>
          <div>
            <button :disabled="busy || !selectedFiles.length" @click="emit('transfer', 'removemany', undefined, [...selectedFiles])">{{ t('取消所选共享') }}<span class="file-selection-count">（{{ selectedFiles.length }}）</span></button>
            <button :disabled="busy || !state.sharedFiles.length" @click="emit('transfer', 'removemany', undefined, state.sharedFiles.map(file => file.id))">{{ t('全部取消共享') }}</button>
          </div>
        </div>
        <div class="shared-file-viewport" :tabindex="state.sharedFiles.length ? 0 : undefined" :aria-label="t('共享文件列表')">
          <ul class="shared-files">
            <li v-for="file in visibleFiles" :key="file.id" :class="{ selected: selectedFiles.includes(file.id) }">
              <label><input v-model="selectedFiles" type="checkbox" :value="file.id" :disabled="busy" :aria-label="t('选择 {0}', file.name)" /><span class="shared-file-copy"><b :title="file.name">{{ file.name }}</b><span class="shared-file-meta"><span class="shared-file-kind">{{ fileKind(file.name) }}</span><small>{{ formatBytes(file.size) }}</small></span></span></label>
              <button :aria-label="t('取消共享 {0}', file.name)" :title="t('取消共享，保留原文件')" :disabled="busy" @click="emit('transfer', 'remove', file.id)"><X aria-hidden="true" /></button>
            </li>
          </ul>
          <div v-if="state.sharedFiles.length && !visibleFiles.length" class="file-empty-state"><SearchX aria-hidden="true" /><p role="status">{{ t('没有匹配的文件') }}</p><button class="text-button" @click="fileQuery = ''">{{ t('清除搜索') }}</button></div>
          <div v-if="!state.sharedFiles.length" class="file-empty-state"><Files aria-hidden="true" /><p>{{ t('暂无共享文件，选择后会显示在这里。') }}</p></div>
        </div>
      </section>
    </div>
  </section>
</template>
