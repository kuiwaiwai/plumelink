<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Monitor, Folder, ChevronDown, Pencil, X } from "@lucide/vue";
import { errorText } from "../../../shared/format";
import { t, displayMessage } from "../../../shared/i18n";
type Entry = { id: string; name: string; path: string; kind?: "application" | "folder" };
type Action = "list" | "add" | "add-folder" | "remove" | "rename";
const entries = ref<Entry[]>([]);
const busy = ref(false);
const error = ref("");
const errorEntry = ref("");
const invalidName = ref(false);
const failedOperation = ref<{ action: Action; id?: string; name?: string }>();
const editing = ref("");
const draft = ref("");
const nameInput = ref<HTMLInputElement>();
const addButton = ref<HTMLButtonElement>();
function bindNameInput(input: unknown) { nameInput.value = input instanceof HTMLInputElement ? input : undefined; }
function clearError() {
  error.value = "";
  errorEntry.value = "";
  invalidName.value = false;
  failedOperation.value = undefined;
}
async function manage(action: Action, id?: string, name?: string) {
  if (busy.value) return false;
  busy.value = true;
  clearError();
  const removedIndex = entries.value.findIndex(entry => entry.id === id);
  let removed = false;
  try {
    entries.value = await invoke<Entry[]>("application_action", { action, id, name });
    if (!entries.value.some(entry => entry.id === editing.value)) editing.value = "";
    removed = action === "remove";
    return true;
  }
  catch (reason) {
    error.value = displayMessage(errorText(reason, t("快捷入口操作失败")));
    if (action === "rename") errorEntry.value = id || "";
    else failedOperation.value = { action, id, name };
    return false;
  }
  finally {
    busy.value = false;
    if (removed) {
      await nextTick();
      const next = entries.value[Math.max(0, Math.min(removedIndex, entries.value.length - 1))];
      if (next) document.getElementById("rename-app-" + next.id)?.focus();
      else addButton.value?.focus();
    }
  }
}
async function retry() {
  const operation = failedOperation.value;
  if (operation) await manage(operation.action, operation.id, operation.name);
}
async function edit(entry: Entry) {
  editing.value = entry.id;
  draft.value = entry.name;
  clearError();
  await nextTick();
  nameInput.value?.focus();
  nameInput.value?.select();
}
async function save() {
  if (busy.value || !editing.value) return;
  const name = draft.value.trim();
  if (!name || name.length > 80 || /[\u0000-\u001f\u007f-\u009f]/.test(name)) {
    error.value = t("快捷入口名称无效，请使用 1–80 个可见字符");
    errorEntry.value = editing.value;
    invalidName.value = true;
    nameInput.value?.focus();
    return;
  }
  const id = editing.value;
  if (await manage("rename", id, name)) await closeEditor(id);
  else {
    await nextTick();
    nameInput.value?.focus();
  }
}
async function closeEditor(id = editing.value) {
  editing.value = "";
  clearError();
  await nextTick();
  document.getElementById("rename-app-" + id)?.focus();
}
onMounted(() => { void manage("list"); });
</script>
<template>
  <details class="settings settings-subsection">
    <summary><span class="section-title"><Monitor aria-hidden="true" />{{ t('电脑快捷入口') }}</span><ChevronDown aria-hidden="true" /></summary>
    <div class="details-body application-settings" :aria-busy="busy">
      <p>{{ t('添加应用或文件夹，在手机快捷操作中选择“快捷启动”。') }}</p>
      <div class="entry-add">
        <button ref="addButton" :disabled="busy || !!editing || entries.length >= 32" :title="t('请选择 Windows 应用的 .exe 文件')" @click="manage('add')"><Monitor aria-hidden="true" />{{ t('添加应用') }}</button>
        <button :disabled="busy || !!editing || entries.length >= 32" @click="manage('add-folder')"><Folder aria-hidden="true" />{{ t('添加文件夹') }}</button>
      </div>
      <p v-if="error && !errorEntry" class="entry-error" role="alert">{{ error }} <button v-if="failedOperation" :disabled="busy" @click="retry">{{ t('重试') }}</button></p>
      <ul v-if="entries.length">
        <li v-for="entry in entries" :key="entry.id">
          <component :is="entry.kind === 'folder' ? Folder : Monitor" class="entry-symbol" aria-hidden="true" />
          <div class="entry-copy">
            <form v-if="editing === entry.id" class="entry-edit" :aria-busy="busy" @submit.prevent="save" @keydown.esc.stop.prevent="!busy && closeEditor()">
              <input :ref="bindNameInput" v-model="draft" :aria-label="t('名称')" :disabled="busy" :aria-invalid="invalidName" :aria-describedby="error && errorEntry === entry.id ? 'entry-error-' + entry.id : undefined" maxlength="80" autocomplete="off" />
              <p v-if="error && errorEntry === entry.id" :id="'entry-error-' + entry.id" class="entry-error" role="alert">{{ error }}</p>
              <div><button type="submit" :disabled="busy">{{ t('保存') }}</button><button type="button" :disabled="busy" @click="closeEditor()">{{ t('取消') }}</button></div>
            </form>
            <b v-else>{{ entry.name }}</b>
            <small>{{ entry.path }}</small>
          </div>
          <div v-if="!editing" class="entry-actions">
            <button :id="'rename-app-' + entry.id" :disabled="busy" :aria-label="t('重命名 {0}', entry.name)" @click="edit(entry)"><Pencil aria-hidden="true" /></button>
            <button :disabled="busy" :aria-label="t('移除 {0}', entry.name)" @click="manage('remove', entry.id)"><X aria-hidden="true" /></button>
          </div>
        </li>
      </ul>
    </div>
  </details>
</template>
<style scoped>
.application-settings { display:grid; gap:12px; container-type:inline-size; }
.entry-add { display:flex; flex-wrap:wrap; align-items:center; gap:8px; }
.entry-add button { display:inline-flex; align-items:center; gap:8px; }
.entry-add svg { width:18px; height:18px; flex-shrink:0; }
ul { list-style:none; margin:0; padding:0; }
li { display:flex; align-items:center; gap:12px; padding:12px 0; border-bottom:1px solid var(--border); }
.entry-symbol { width:20px; height:20px; flex-shrink:0; color:var(--muted); }
.entry-copy { flex:1; min-width:0; display:grid; gap:4px; }
.entry-copy b { overflow-wrap:anywhere; }
.entry-actions,.entry-edit > div { display:flex; flex-shrink:0; gap:4px; }
.entry-actions button { padding:6px; }
.entry-edit { display:grid; gap:8px; }
.entry-error { color:var(--danger-text); overflow-wrap:anywhere; line-height:1.5; }
.entry-edit input { width:100%; min-width:0; box-sizing:border-box; padding:8px 10px; border:1px solid var(--border); border-radius:8px; background:var(--surface); color:var(--text); }
small { color:var(--muted); overflow-wrap:anywhere; }
@container (max-width:320px) {
  li { display:grid; grid-template-columns:20px minmax(0,1fr); align-items:start; gap:8px 12px; }
  .entry-symbol { margin-top:2px; }
  .entry-copy,.entry-actions { grid-column:2; }
  .entry-actions { justify-self:end; }
}
</style>
