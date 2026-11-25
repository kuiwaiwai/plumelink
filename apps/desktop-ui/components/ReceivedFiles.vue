<script setup lang="ts">
import { computed, ref } from "vue";
import { ChevronDown, Download, FolderOpen, Trash2, X } from "@lucide/vue";
import { displayMessage, t } from "../../../shared/i18n";
import { errorText, formatBytes } from "../../../shared/format";
import type { ReceivedFile, ReceiveAction } from "../../../shared/protocol";

const props = defineProps<{
  files: ReceivedFile[];
  directory: string;
  busy: boolean;
  action: (action: ReceiveAction, taskId?: string) => Promise<void>;
}>();
const emit = defineEmits<{ pickDirectory: [] }>();
const expanded = ref(true);
const opening = ref(false);
const clearing = ref(false);
const cancelling = ref(new Set<string>());
const error = ref("");
const finished = (file: ReceivedFile) => file.state === "completed" || file.state === "stopped";
const activeCount = computed(() => props.files.filter(file => !finished(file)).length);
const hasFinished = computed(() => props.files.some(finished));
const status = (file: ReceivedFile) => ({
  receiving: t("正在接收"), saving: t("正在保存"), stopping: t("正在停止"),
  completed: t("已保存"), stopped: t("已停止"),
})[file.state];
const percent = (file: ReceivedFile) => file.size > 0
  ? Math.min(100, Math.max(0, file.receivedBytes / file.size * 100)) : 0;

async function run(action: ReceiveAction, taskId?: string) {
  if (action === "open" && opening.value || action === "clear" && clearing.value ||
      action === "cancel" && (!taskId || cancelling.value.has(taskId))) return;
  error.value = "";
  if (action === "open") opening.value = true;
  if (action === "clear") clearing.value = true;
  if (action === "cancel") cancelling.value.add(taskId!);
  try { await props.action(action, taskId); }
  catch (cause) { error.value = errorText(cause, t("操作失败")); }
  finally {
    if (action === "open") opening.value = false;
    if (action === "clear") clearing.value = false;
    if (action === "cancel") cancelling.value.delete(taskId!);
  }
}
</script>

<template>
  <section class="received-panel" :aria-label="t('手机发来的文件')">
    <div class="transfer-setting">
      <span><b>{{ t('Windows 接收目录') }}</b><small :title="directory">{{ directory }}</small></span>
      <div class="receive-folder-actions">
        <button :disabled="opening || !directory" @click="run('open')"><FolderOpen aria-hidden="true" />{{ t('打开目录') }}</button>
        <button :disabled="busy" @click="emit('pickDirectory')">{{ t('选择目录') }}</button>
      </div>
    </div>
    <p v-if="error" class="error receive-error" role="alert">{{ displayMessage(error) }}</p>
    <details :open="expanded" @toggle="expanded = ($event.target as HTMLDetailsElement).open">
      <summary><Download aria-hidden="true" /><b>{{ t('手机发来的文件') }}</b><span v-if="activeCount" class="receive-count">{{ activeCount }}</span><ChevronDown class="receive-chevron" aria-hidden="true" /></summary>
      <div class="receive-content">
        <div v-if="hasFinished" class="receive-toolbar"><button :disabled="clearing" :title="t('只清除记录，已保存的文件不受影响')" @click="run('clear')"><Trash2 aria-hidden="true" />{{ t('清空已结束记录') }}</button></div>
        <ul v-if="files.length" class="receive-list" :aria-label="t('接收记录')" tabindex="0">
          <li v-for="file in files" :key="file.taskId" :data-task="file.taskId" :data-state="file.state">
            <div class="receive-file-copy">
              <b class="receive-name" :title="file.name">{{ file.name }}</b>
              <div class="receive-meta"><span class="receive-device" :title="file.deviceName">{{ file.deviceName || t('移动设备') }}</span><span>{{ finished(file) ? formatBytes(file.receivedBytes) : `${formatBytes(file.receivedBytes)} / ${formatBytes(file.size)}` }}</span><span class="receive-status">{{ status(file) }}</span></div>
              <progress v-if="!finished(file)" :value="percent(file)" max="100" :aria-label="t('接收进度 {0}', file.name)" />
              <p v-if="file.error" class="receive-file-error">{{ displayMessage(file.error) }}</p>
            </div>
            <button v-if="file.state === 'completed'" class="receive-row-action" :disabled="opening" :aria-label="t('打开 {0} 所在目录', file.name)" :title="t('打开所在目录')" @click="run('open', file.taskId)"><FolderOpen aria-hidden="true" /></button>
            <button v-else-if="!finished(file)" class="receive-row-action" :disabled="cancelling.has(file.taskId) || file.state === 'stopping' && !file.error" :aria-label="t('停止接收 {0}', file.name)" :title="file.state === 'stopping' && file.error ? t('重试停止') : t('停止接收')" @click="run('cancel', file.taskId)"><X aria-hidden="true" /></button>
          </li>
        </ul>
        <p v-else class="receive-empty">{{ t('暂无接收记录') }}</p>
      </div>
    </details>
  </section>
</template>

<style scoped>
.received-panel { min-width: 0; }
.transfer-setting { padding: 4px 0 16px; }
.transfer-setting > span { flex: 1; }
.transfer-setting small { line-height: 1.5; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.receive-folder-actions { display: flex; flex-wrap: wrap; gap: 8px; }
details { border: 1px solid var(--edge); border-radius: 14px; background: var(--surface); overflow: hidden; }
summary { display: flex; align-items: center; gap: 10px; padding: 14px 16px; cursor: pointer; list-style: none; min-height: 48px; }
summary::-webkit-details-marker { display: none; }
summary > svg { width: 18px; height: 18px; flex-shrink: 0; color: var(--muted); }
details[open] > summary { border-bottom: 0; }
details[open] > summary > svg:first-child { transform: none; }
.receive-count { background: var(--surface-2); color: var(--accent); border-radius: 20px; padding: 2px 8px; font-size: 12px; }
.receive-chevron { margin-left: auto; transition: transform .15s; }
details[open] .receive-chevron { transform: rotate(180deg); }
.receive-content { border-top: 1px solid var(--edge); }
.receive-toolbar { display: flex; justify-content: flex-end; padding: 10px 14px 0; }
.receive-toolbar button { font-size: 12px; }
.receive-list { list-style: none; margin: 0; padding: 0 16px; max-height: 320px; overflow-y: auto; overscroll-behavior: contain; }
.receive-list li { display: flex; align-items: center; gap: 12px; padding: 14px 0; }
.receive-list li + li { border-top: 1px solid var(--edge); }
.receive-file-copy { flex: 1; min-width: 0; }
.receive-name { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; overflow: hidden; overflow-wrap: anywhere; font-size: 13px; line-height: 1.5; }
.receive-meta { display: flex; flex-wrap: wrap; gap: 4px 12px; color: var(--muted); font-size: 12px; margin-top: 5px; line-height: 1.5; }
.receive-device { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 180px; }
.receive-status { margin-left: auto; }
li[data-state="completed"] .receive-status { color: var(--success); }
progress { display: block; appearance: none; width: 100%; height: 4px; border: 0; border-radius: 4px; margin-top: 10px; overflow: hidden; background: var(--surface-2); }
progress::-webkit-progress-bar { background: var(--surface-2); }
progress::-webkit-progress-value { background: var(--accent); border-radius: 4px; }
.receive-row-action { width: 36px; height: 36px; min-width: 36px; padding: 8px; }
.receive-row-action svg { width: 18px; height: 18px; }
.receive-empty { color: var(--muted); text-align: center; margin: 0; padding: 24px 16px; }
.receive-file-error { color: var(--danger-text); font-size: 12px; margin: 7px 0 0; overflow-wrap: anywhere; }
.receive-error { margin: 0 0 12px; }
@media (max-width: 650px) { .receive-folder-actions { width: 100%; } }
@media (prefers-reduced-motion: reduce) { .receive-chevron { transition: none; } }
</style>
