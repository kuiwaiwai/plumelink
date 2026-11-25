import { computed, onMounted, onUnmounted, ref, type Ref } from "vue";
import { androidBridge } from "../native/bridge";

export type SharedSelectionFile = { id: string; name: string; size: number; pending: boolean; error: string };
type Selection = { revision: string; files: SharedSelectionFile[]; message: string };
const idPattern = /^[a-f0-9]{32}$/;

export function readSharedSelection(value: unknown): Selection | undefined {
  if (!value || typeof value !== "object") return;
  const state = value as Record<string, unknown>;
  if (typeof state.revision !== "string" || !idPattern.test(state.revision) || !Array.isArray(state.files)
    || state.files.length > 16 || typeof state.message !== "string" || state.message.length > 512) return;
  const ids = new Set<string>();
  const files: SharedSelectionFile[] = [];
  for (const item of state.files) {
    if (!item || typeof item !== "object") return;
    const file = item as Record<string, unknown>;
    if (typeof file.id !== "string" || !idPattern.test(file.id) || ids.has(file.id)
      || typeof file.name !== "string" || file.name.length > 512 || /[\u0000-\u001f\u007f]/.test(file.name)
      || typeof file.size !== "number" || !Number.isSafeInteger(file.size) || file.size < -1
      || file.size > 2 * 1024 ** 3 + 1 || typeof file.pending !== "boolean"
      || typeof file.error !== "string" || file.error.length > 512) return;
    ids.add(file.id);
    files.push({ id: file.id, name: file.name, size: file.size, pending: file.pending, error: file.error });
  }
  return { revision: state.revision, files, message: state.message };
}

export function useSharedFiles(canStart: Readonly<Ref<boolean>>) {
  const android = androidBridge();
  const selection = ref<Selection>({ revision: "", files: [], message: "" });
  const openRequest = ref(0);
  const submitting = ref(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const canSend = computed(() => !!android?.sendSharedFiles && canStart.value && !submitting.value
    && selection.value.files.length > 0 && selection.value.files.every(file => !file.pending && !file.error));
  function accept(value: unknown, open = false) {
    const parsed = readSharedSelection(value);
    if (!parsed) return;
    selection.value = parsed;
    submitting.value = false; clearTimeout(timer);
    if (open) openRequest.value++;
  }
  function event(event: Event) {
    const value = (event as CustomEvent<unknown>).detail;
    accept(value, !!value && typeof value === "object" && (value as { open?: unknown }).open === true);
  }
  function sync(initial = false) {
    try {
      const value = JSON.parse(android?.getSharedFiles?.() || "null");
      accept(value, initial && Array.isArray(value?.files) && (value.files.length > 0 || !!value.message));
    } catch {}
  }
  function send() {
    if (!canSend.value) return;
    submitting.value = true;
    timer = setTimeout(() => { submitting.value = false; sync(); }, 8000);
    try { android?.sendSharedFiles?.(selection.value.revision); }
    catch { submitting.value = false; clearTimeout(timer); }
  }
  function canRetry(file: SharedSelectionFile) {
    return !!android?.retrySharedFile && !submitting.value && !selection.value.files.some(item => item.pending)
      && selection.value.files.some(item => item.id === file.id && !item.pending && !!item.error);
  }
  function retry(file: SharedSelectionFile) {
    if (!canRetry(file)) return;
    submitting.value = true;
    timer = setTimeout(() => { submitting.value = false; sync(); }, 8000);
    try { android?.retrySharedFile?.(selection.value.revision, file.id); }
    catch { submitting.value = false; clearTimeout(timer); }
  }
  function remove(file: SharedSelectionFile) {
    if (!selection.value.files.some(item => item.id === file.id)) return;
    android?.removeSharedFile?.(selection.value.revision, file.id);
  }
  function clear() { android?.clearSharedFiles?.(selection.value.revision); }
  const resume = () => sync();
  onMounted(() => {
    window.addEventListener("plumelink-shared-files", event);
    window.addEventListener("pageshow", resume);
    sync(true);
  });
  onUnmounted(() => {
    clearTimeout(timer);
    window.removeEventListener("plumelink-shared-files", event);
    window.removeEventListener("pageshow", resume);
  });
  return { selection, openRequest, canSend, submitting, send, canRetry, retry, remove, clear };
}
