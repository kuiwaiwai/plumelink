<script setup lang="ts">
import { displayMessage, t } from "../../../shared/i18n";
import { formatBytes } from "../../../shared/format";

import { computed } from "vue";
import { FolderOpen, Upload, Download, ArrowDownUp } from "@lucide/vue";
type SharedFile = { id: number; name: string; size: number };
const props = defineProps<{ native: boolean; canFiles: boolean; androidFolder: string; folderReady: boolean; sharedFiles: SharedFile[]; filesLoading: boolean; filesError: string; transferBusy: boolean; transferCancelling: boolean; transferProgress: number; transferMessage: string; transferHistory: string[] }>();
const progress = computed(() => Number.isFinite(props.transferProgress) ? Math.max(0, Math.min(100, Math.round(props.transferProgress))) : 0);
const stage = computed(() => props.transferCancelling ? t('正在取消传输') : progress.value === 0 ? t('正在准备传输') : progress.value === 100 ? t('正在确认完成') : t('正在传输文件'));
defineEmits<{ folder: []; upload: [event: Event]; refresh: []; download: [file: SharedFile]; cancel: []; clearHistory: [] }>();
</script>
<template>
    <section class="file-panel page-scroll" :aria-label="t('文件传输')" :aria-busy="transferBusy || filesLoading">
      <p v-if="!canFiles" class="inline-notice" role="status">{{ t('连接电脑并获得文件权限后，即可开始传输。') }}</p>
      <div class="file-actions">
        <div class="file-folder"><span><b>{{ t('Android 接收目录') }}</b><small :title="androidFolder" :aria-label="androidFolder">{{ androidFolder }}</small></span><button v-if="native" :disabled="transferBusy || !canFiles" @click="$emit('folder')"><FolderOpen aria-hidden="true" />{{ t('选择目录') }}</button></div>
        <p v-if="native && !folderReady && canFiles" class="inline-notice" role="status">{{ t('请先选择 Android 接收目录，再保存电脑文件。') }}</p>
        <div v-if="transferBusy" class="transfer-progress" :class="{ cancelling: transferCancelling }" role="region" :aria-label="t('当前文件传输')">
          <div class="transfer-heading"><span class="transfer-symbol"><ArrowDownUp aria-hidden="true" /></span><span class="transfer-stage" role="status">{{ stage }}</span><strong class="transfer-percent">{{ progress }}<small>%</small></strong></div>
          <progress :value="progress" max="100" :aria-label="t('文件传输进度')"></progress>
          <div class="transfer-detail"><p class="transfer-description">{{ displayMessage(transferMessage) || t('正在与电脑建立传输连接…') }}</p><button class="transfer-cancel" :disabled="transferCancelling" @click="$emit('cancel')">{{ transferCancelling ? t('取消中…') : t('取消传输') }}</button></div>
        </div>
        <label class="file-upload" :class="{ disabled: !canFiles || transferBusy }" :aria-disabled="!canFiles || transferBusy"><Upload aria-hidden="true" /><span>{{ transferBusy ? t('传输中…') : t('发送文件到 Windows') }}</span><input type="file" multiple :disabled="!canFiles || transferBusy" @change="$emit('upload', $event)" /></label>
        <p class="file-security">{{ t('单文件最大 2 GB') }}</p>
        <p v-if="transferMessage && !transferBusy" class="panel-feedback" role="status">{{ displayMessage(transferMessage) }}</p>
        <details v-if="transferHistory.length" class="transfer-history"><summary>{{ t('最近传输') }}</summary><ul><li v-for="item in transferHistory" :key="item">{{ item }}</li></ul><button @click="$emit('clearHistory')">{{ t('清空记录') }}</button></details>
      </div>
      <div class="file-library">
        <div class="phone-files" :aria-busy="filesLoading"><h3>{{ t('Windows 共享的文件') }}</h3><button class="refresh-files" :disabled="!canFiles || transferBusy || filesLoading" @click="$emit('refresh')">{{ t('刷新') }}</button></div>
        <p v-if="canFiles && filesError" class="panel-feedback" role="alert">{{ displayMessage(filesError) }} <button class="text-button" :disabled="transferBusy || filesLoading" @click="$emit('refresh')">{{ filesLoading ? t('读取中…') : t('重试') }}</button></p>
        <div v-if="canFiles && filesLoading && !sharedFiles.length" class="empty-state" role="status" aria-live="polite"><FolderOpen aria-hidden="true" /><h2>{{ t('正在读取共享文件…') }}</h2><p>{{ t('请稍候。') }}</p></div>
        <div v-else-if="canFiles && !sharedFiles.length && !filesError" class="empty-state"><FolderOpen aria-hidden="true" /><h2>{{ t('电脑文件会显示在这里') }}</h2><p>{{ t('在电脑端选择要共享的文件，即可保存到手机。') }}</p></div>
        <ul v-if="canFiles && sharedFiles.length" class="file-list"><li v-for="file in sharedFiles" :key="file.id"><span><b>{{ file.name }}</b><small>{{ formatBytes(file.size) }}</small></span><button :disabled="!canFiles || transferBusy || (native && !folderReady)" :aria-label="t('保存 {0}', file.name)" @click="$emit('download', file)"><Download aria-hidden="true" />{{ t('保存') }}</button></li></ul>
      </div>
    </section>

</template>
