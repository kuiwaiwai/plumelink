import { formatBytes } from "../../../shared/format";
import { locale, t } from "../../../shared/i18n";
import { computed, ref, onMounted, onUnmounted, watch, type Ref } from "vue";
import { storage } from "../storage";
import { androidBridge } from "../native/bridge";
import { uploadResumable, MAX_FILE_BYTES, isSafeFileName } from "../file-transfer";
import type { useConnection } from "../connection";
import { formatTransferHistory, nativeTransferHistory, readTransferHistory, type TransferHistoryEntry } from "../../../shared/transfer-history";

type TransferConnection = Pick<
  ReturnType<typeof useConnection>,
  "api" | "token" | "latency" | "fileAccessPending"
>;
export type UnfinishedDownload = { id: string; name: string; size: number; expected: number; available: boolean; version: string; folder?: string; pendingCommit?: boolean };
export function downloadRecoveryStatus(file: UnfinishedDownload) {
  if (file.pendingCommit) return t("保存结果待确认");
  return file.available ? t("已接收 {0} / {1}", file.size < 0 ? "—" : formatBytes(file.size), formatBytes(file.expected)) : t("文件暂时无法访问");
}
export function downloadRecoveryLabel(files: readonly UnfinishedDownload[]) {
  return files.some(file => file.pendingCommit) ? t("{0} 个待核对文件", files.length) : t("{0} 个未完成文件", files.length);
}
const MAX_FOLDER_LABEL_LENGTH = 8192;
export function useFileTransfers(
  remote: TransferConnection,
  canFiles: Readonly<Ref<boolean>>,
  microphoneActive: Readonly<Ref<boolean>>,
) {
  const android = androidBridge();
  type SharedFile = { id: number; name: string; size: number };
  const sharedFiles = ref<SharedFile[]>([]);
  const filesLoading = ref(false);
  const filesError = ref("");
  const transferBusy = ref(false);
  const transferCancelling = ref(false);
  const transferProgress = ref(0);
  const transferMessage = ref("");
  const unfinishedDownloads = ref<UnfinishedDownload[]>([]);
  const recoveryBusy = ref(false);
  const recoveryMessage = ref("");
  let recoveryRequest = "";
  let recoveryTimer: ReturnType<typeof setTimeout> | undefined;
  function resolveUnfinishedDownload(file: UnfinishedDownload, keep: boolean) {
    if (recoveryBusy.value || transferBusy.value || !android?.resolveUnfinishedDownload) return;
    recoveryRequest = Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, "0")).join("");
    recoveryBusy.value = true;
    recoveryMessage.value = "";
    recoveryTimer = setTimeout(() => {
      recoveryBusy.value = false;
      recoveryRequest = "";
      recoveryMessage.value = t("处理超时，请刷新后核对文件。");
    }, 15_000);
    try { android.resolveUnfinishedDownload(file.id, file.version, keep, recoveryRequest); }
    catch (error) {
      clearTimeout(recoveryTimer); recoveryBusy.value = false; recoveryRequest = "";
      recoveryMessage.value = error instanceof Error ? error.message : t("操作失败");
    }
  }
  function nativeRecovery(event: Event) {
    const value = (event as CustomEvent<unknown>).detail;
    if (!value || typeof value !== "object") return;
    const detail = value as { files?: unknown; message?: unknown; request?: unknown };
    if (typeof detail.request === "string" && detail.request !== recoveryRequest) return;
    if (detail.request === recoveryRequest && recoveryRequest) {
      recoveryBusy.value = false;
      recoveryRequest = "";
      clearTimeout(recoveryTimer);
    }
    if (Array.isArray(detail.files)) {
      const ids = new Set<string>();
      unfinishedDownloads.value = detail.files.slice(0, 64).filter((file): file is UnfinishedDownload => {
        if (!file || typeof file !== "object") return false;
        const item = file as Record<string, unknown>;
        if (typeof item.id !== "string" || !/^[a-f0-9]{32}$/.test(item.id) || ids.has(item.id)
          || typeof item.name !== "string" || !item.name.length || item.name.length > 512 || /[\u0000-\u001f\u007f]/.test(item.name)
          || !Number.isSafeInteger(item.size) || (item.size as number) < -1 || !Number.isSafeInteger(item.expected)
          || (item.expected as number) < 0 || (item.expected as number) > MAX_FILE_BYTES
          || typeof item.available !== "boolean" || typeof item.version !== "string"
          || (item.pendingCommit !== undefined && typeof item.pendingCommit !== "boolean")
          || (item.folder !== undefined && (typeof item.folder !== "string" || item.folder.length > 1024))
          || !(item.version === "" || /^[a-f0-9]{64}$/.test(item.version))) return false;
        ids.add(item.id); return true;
      });
    }
    if (!recoveryBusy.value && typeof detail.message === "string" && detail.message.length <= 512) recoveryMessage.value = detail.message;
  }
  let successTimer: ReturnType<typeof setTimeout> | undefined;
  let successMessage = "";
  function clearSuccess() {
    clearTimeout(successTimer);
    if (successMessage && transferMessage.value === successMessage) transferMessage.value = "";
    successMessage = "";
  }
  function showSuccess(message: string) {
    clearSuccess();
    if (!canFiles.value) return;
    successMessage = transferMessage.value = message;
    successTimer = setTimeout(clearSuccess, 3000);
  }
  const historyEntries = ref<TransferHistoryEntry[]>([]);
  const transferHistory = computed(() => historyEntries.value.map(formatTransferHistory));
  try {
    const history: unknown = JSON.parse(
      storage.getItem("plumelink-transfer-history") || "[]",
    );
    historyEntries.value = readTransferHistory(history);
  } catch {}
  let transferAbort: AbortController | null = null;
  let transferCancelled = false;
  let filesRequestSerial = 0;
  let filesAbort: AbortController | undefined;
  let refreshAfterTransfer = false;
  let ignoreLateNativeEvents = false;
  const nativeTransferBlocked = ref<"stale" | "reconnecting" | false>(false);
  const canStartTransfer = computed(() => canFiles.value && !transferBusy.value && !nativeTransferBlocked.value);
  let nativeWatchdog: ReturnType<typeof setTimeout> | undefined;
  let cancelWatchdog: ReturnType<typeof setTimeout> | undefined;
  const clearNativeWatchdog = () => {
    clearTimeout(nativeWatchdog);
    nativeWatchdog = undefined;
  };
  const clearCancelWatchdog = () => {
    clearTimeout(cancelWatchdog);
    cancelWatchdog = undefined;
  };
  const armNativeWatchdog = () => {
    clearNativeWatchdog();
    nativeWatchdog = setTimeout(() => {
      nativeWatchdog = undefined;
      if (!transferBusy.value) return;
      android?.cancelFileTransfer();
      transferBusy.value = false;
      transferCancelling.value = false;
      ignoreLateNativeEvents = true;
      nativeTransferBlocked.value = "stale";
      transferMessage.value = t('文件传输长时间没有响应，已请求停止，请稍后刷新');
      refreshFilesAfterTransfer();
    }, 15 * 60 * 1000);
  };
  const armCancelWatchdog = () => {
    clearCancelWatchdog();
    cancelWatchdog = setTimeout(() => {
      cancelWatchdog = undefined;
      if (!transferBusy.value) return;
      transferBusy.value = false;
      transferCancelling.value = false;
      ignoreLateNativeEvents = true;
      nativeTransferBlocked.value = "stale";
      transferMessage.value = t('取消尚未完成，已暂停新传输，请稍后刷新');
      refreshFilesAfterTransfer();
    }, 15_000);
  };
  const receiveFolder = () => android?.getReceiveFolder() || t('浏览器默认下载目录');
  const androidFolder = ref(receiveFolder());
  const folderReady = ref(!android || android.hasReceiveFolder?.() !== false);
  watch(locale, () => { androidFolder.value = receiveFolder(); });
  function rememberTransfer(entry: TransferHistoryEntry) {
    const serialized = JSON.stringify(entry);
    historyEntries.value = [
      entry,
      ...historyEntries.value.filter(item => JSON.stringify(item) !== serialized),
    ].slice(0, 10);
    try {
      storage.setItem(
        "plumelink-transfer-history",
        JSON.stringify(historyEntries.value),
      );
    } catch {}
  }
  function clearTransferHistory() {
    historyEntries.value = [];
    storage.removeItem("plumelink-transfer-history");
  }
  async function loadFiles() {
    try { android?.refreshReceiveFolder?.(); } catch {}
    const serial = ++filesRequestSerial;
    filesAbort?.abort();
    const request = new AbortController();
    filesAbort = request;
    filesLoading.value = true;
    filesError.value = "";
    if (!canFiles.value) {
      sharedFiles.value = [];
      filesLoading.value = false;
      return;
    }
    if (nativeTransferBlocked.value) android?.syncSession?.();
    try {
      const response = await remote.api("/api/files", { signal: request.signal });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || t('无法读取文件列表'));
      if (serial !== filesRequestSerial) return;
      if (!canFiles.value) {
        sharedFiles.value = [];
        return;
      }
      const seenIds = new Set<number>();
      sharedFiles.value = Array.isArray(result.files)
        ? result.files.filter((file: unknown): file is SharedFile => {
            if (!file || typeof file !== "object") return false;
            const item = file as Record<string, unknown>;
            const id = item.id as number;
            const name = item.name as string;
            const size = item.size as number;
            if (!Number.isSafeInteger(id) || id < 0 || seenIds.has(id)) return false;
            if (!isSafeFileName(name)) return false;
            if (!Number.isSafeInteger(size) || size < 0 || size > MAX_FILE_BYTES) return false;
            seenIds.add(id);
            return true;
          })
        : [];
    } catch (e) {
      if (serial !== filesRequestSerial) return;
      filesError.value = e instanceof Error ? e.message : t('无法读取文件列表');
      if (!canFiles.value) sharedFiles.value = [];
    } finally {
      if (serial === filesRequestSerial) { filesLoading.value = false; filesAbort = undefined; }
    }
  }
  function refreshFilesAfterTransfer() {
    if (!refreshAfterTransfer || !canFiles.value) return;
    refreshAfterTransfer = false;
    void loadFiles();
  }
  async function uploadFiles(event: Event) {
    const input = event.target as HTMLInputElement;
    const files = Array.from(input.files || []);
    input.value = "";
    if (!files.length || transferBusy.value) return;
    if (nativeTransferBlocked.value) {
      transferMessage.value = t('上一项传输仍在停止，请稍后刷新');
      return;
    }
    if (files.length > 16) {
      transferMessage.value = t('每次最多选择 16 个文件');
      return;
    }
    const invalidName = files.find(file => !isSafeFileName(file.name));
    if (invalidName) {
      transferMessage.value = t('文件名无效，请重命名后重试');
      return;
    }
    const invalidSize = files.find(file => !Number.isSafeInteger(file.size) || file.size < 0 || file.size > MAX_FILE_BYTES);
    if (invalidSize) {
      transferMessage.value = t('{0} 大小无效或超过 2 GB', invalidSize.name);
      return;
    }
    clearSuccess();
    let completedFiles = 0;
    let lastSavedName = "";
    const tokenAtSelection = remote.token();
    transferCancelled = false;
    ignoreLateNativeEvents = false;
    clearCancelWatchdog();
    filesError.value = "";
    transferBusy.value = true;
    transferCancelling.value = false;
    transferProgress.value = 0;
    try {
      // Android's picker pauses the Activity; resume this same connection first.
      if (!canFiles.value) {
        transferMessage.value = t('正在恢复与电脑的连接…');
        for (
          let attempt = 0;
          attempt < 40 &&
          !canFiles.value &&
          !transferCancelled &&
          tokenAtSelection === remote.token();
          attempt++
        )
          await new Promise((resolve) => setTimeout(resolve, 100));
      }
      if (transferCancelled) throw new Error(t('已取消传输'));
      if (!canFiles.value || tokenAtSelection !== remote.token())
        throw new Error(t('连接未恢复，文件未发送，请重试'));
      for (const file of files) {
        if (transferCancelled) throw new Error(t('已取消传输'));
        transferProgress.value = Math.floor(completedFiles * 100 / files.length);
        transferMessage.value = t('正在发送 {0}…', file.name);
        transferAbort = new AbortController();
        const startedAt = Date.now();
        const formatRate = (sentBytes: number) => {
          const elapsedSec = (Date.now() - startedAt) / 1000;
          if (elapsedSec < 0.4 || sentBytes <= 0) return "";
          const bps = sentBytes / elapsedSec;
          const speed =
            bps >= 1048576
              ? `${(bps / 1048576).toFixed(1)} MB/s`
              : `${(bps / 1024).toFixed(0)} KB/s`;
          const etaSec = Math.max(1, Math.round((file.size - sentBytes) / bps));
          const eta =
            etaSec >= 60
              ? t('剩余 {0} 分 {1} 秒', Math.floor(etaSec / 60), etaSec % 60)
              : t('剩余 {0} 秒', etaSec);
          return `（${speed} · ${eta}）`;
        };
        const name = await uploadResumable(file, {
          token: tokenAtSelection,
          signal: transferAbort.signal,
          valid: () =>
            !transferCancelled && remote.token() === tokenAtSelection,
          realtime: () => microphoneActive.value,
          latency: () => remote.latency.value,
          progress: (bytes, retrying) => {
            const fraction = file.size === 0 ? 1 : Math.max(0, Math.min(1, bytes / file.size));
            transferProgress.value = Math.min(99, Math.floor((completedFiles + fraction) * 100 / files.length));
            transferMessage.value = retrying
              ? t('网络中断，正在续传 {0}…', file.name)
              : t('正在发送 {0}{1}…', file.name, formatRate(bytes));
          },
        });
        completedFiles++;
        lastSavedName = name;
        rememberTransfer({ kind: "sent", name });
      }
      transferProgress.value = 100;
      showSuccess(files.length === 1 ? t('{0} 已保存到电脑', lastSavedName) : t('已发送 {0} 个文件', completedFiles));
    } catch (e) {
      const reason = e instanceof Error ? e.message : t('文件发送失败');
      transferMessage.value = completedFiles > 0
        ? t('已发送 {0}/{1} 个文件。{2}', completedFiles, files.length, reason) : reason;
    } finally {
      transferAbort = null;
      transferBusy.value = false;
      transferCancelling.value = false;
      clearCancelWatchdog();
      refreshFilesAfterTransfer();
    }
  }
  async function downloadFile(file: SharedFile) {
    if (transferBusy.value || !canFiles.value) return;
    if (android && android.hasReceiveFolder?.() === false) {
      folderReady.value = false;
      transferMessage.value = t('请先选择 Android 接收目录，再保存电脑文件。');
      return;
    }
    if (nativeTransferBlocked.value) {
      transferMessage.value = t('上一项传输仍在停止，请稍后刷新');
      return;
    }
    if (
      !file ||
      !Number.isSafeInteger(file.id) ||
      file.id < 0 ||
      !isSafeFileName(file.name) ||
      !Number.isSafeInteger(file.size) ||
      file.size < 0 ||
      file.size > MAX_FILE_BYTES
    ) {
      transferMessage.value = t('文件信息无效，请刷新后重试');
      return;
    }
    if (android) {
      clearCancelWatchdog();
      ignoreLateNativeEvents = false;
      filesError.value = "";
      transferBusy.value = true;
      transferCancelling.value = false;
      transferMessage.value = t('正在接收 {0}…', file.name);
      transferProgress.value = 0;
      try {
        android.downloadFile(`/api/files/${file.id}`, file.name, remote.token());
        if (transferBusy.value) armNativeWatchdog();
      } catch (error) {
        transferBusy.value = false;
        transferMessage.value = error instanceof Error ? error.message : t('无法开始文件下载');
      }
      return;
    }
    transferMessage.value = t('请在 Android 应用中保存文件');
  }
  function cancelTransfer() {
    if (!transferBusy.value || transferCancelling.value) return;
    transferCancelled = true;
    transferCancelling.value = true;
    transferAbort?.abort();
    android?.cancelFileTransfer();
    clearNativeWatchdog();
    clearCancelWatchdog();
    transferMessage.value = t('正在取消传输…');
    if (android && transferBusy.value) armCancelWatchdog();
  }
  function chooseUploadFiles() {
    if (!canStartTransfer.value) {
      if (nativeTransferBlocked.value) transferMessage.value = t('上一项传输仍在停止，请稍后刷新');
      return;
    }
    android?.chooseUploadFiles?.();
  }
  const mayContinueNativeTransfer = () => !!android && !transferAbort && remote.fileAccessPending.value;
  watch([canFiles, remote.fileAccessPending], ([allowed], [wasAllowed]) => {
    if (allowed) {
      if (wasAllowed) return;
      // A transfer may have finished while access was revoked. Ask Android for its current state.
      android?.syncSession?.();
      if (!transferBusy.value) {
        refreshAfterTransfer = false;
        void loadFiles();
      } else refreshAfterTransfer = true;
      return;
    }
    refreshAfterTransfer = false;
    filesRequestSerial++;
    filesAbort?.abort();
    filesAbort = undefined;
    filesLoading.value = false;
    filesError.value = "";
    sharedFiles.value = [];
    clearSuccess();
    if (mayContinueNativeTransfer()) return;
    const wasTransferring = transferBusy.value;
    transferCancelled = true;
    transferAbort?.abort();
    if (android && !transferAbort && transferBusy.value) nativeTransferBlocked.value = "reconnecting";
    android?.cancelFileTransfer();
    clearNativeWatchdog();
    if (transferBusy.value && !transferAbort) transferBusy.value = false;
    transferCancelling.value = false;
    if (wasTransferring)
      transferMessage.value = t('连接已断开或文件权限已撤销，传输已停止');
  });
  function nativeTransfer(event: Event) {
    const value = (event as CustomEvent<unknown>).detail;
    const detail = value && typeof value === "object" ? value as {
      message?: string;
      folder?: string;
      progress?: number;
      done?: boolean;
      succeeded?: boolean;
      snapshot?: boolean;
      direction?: unknown;
      fileName?: unknown;
      fileCount?: unknown;
    } : null;
    if (!detail) return;
    if (typeof detail.folder === "string" && detail.folder.length <= MAX_FOLDER_LABEL_LENGTH) {
      androidFolder.value = detail.folder;
      folderReady.value = !android || android.hasReceiveFolder?.() !== false;
    }
    if (!canFiles.value && !(transferBusy.value && mayContinueNativeTransfer())) return;
    const isSnapshot = detail.snapshot === true;
    if (nativeTransferBlocked.value === "stale") {
      if (detail.done !== true) return;
      nativeTransferBlocked.value = false;
      ignoreLateNativeEvents = false;
      // A stale completion clears the task guard but must leave the displayed error intact.
      refreshFilesAfterTransfer();
      return;
    }
    if (nativeTransferBlocked.value === "reconnecting") {
      if (!isSnapshot) return;
      nativeTransferBlocked.value = false;
      ignoreLateNativeEvents = false;
    }
    // Discard late progress and snapshots after cancellation or timeout.
    if (isSnapshot && ignoreLateNativeEvents) return;
    if (!isSnapshot && ignoreLateNativeEvents) return;
    if (isSnapshot) ignoreLateNativeEvents = false;
    if (detail.done === true && detail.succeeded === false && detail.direction === "send"
      && typeof detail.fileCount === "number" && Number.isSafeInteger(detail.fileCount)
      && detail.fileCount >= 1 && detail.fileCount <= 16 && (!isSnapshot || transferBusy.value)) {
      const completed = nativeTransferHistory(detail, "");
      if (completed) rememberTransfer(completed);
    }
    if (typeof detail.message === "string" && detail.message.length <= 512) {
      const succeeded = detail.done !== false && (detail.succeeded === true ||
        (detail.succeeded === undefined && (detail.message.includes('已保存') || detail.message.includes('已发送'))));
      if (succeeded) {
        if (!isSnapshot || transferBusy.value) {
          showSuccess(detail.message);
          rememberTransfer(nativeTransferHistory(detail, detail.message));
        }
      } else if (detail.message || !isSnapshot) {
        clearSuccess();
        transferMessage.value = detail.message;
      }
    }
    if (typeof detail.progress === "number" && Number.isFinite(detail.progress))
      transferProgress.value = Math.max(0, Math.min(100, Math.round(detail.progress)));
    if (typeof detail.done === "boolean") {
      transferBusy.value = !detail.done;
      if (detail.done) transferCancelling.value = false;
      if (detail.done) {
        clearNativeWatchdog();
        clearCancelWatchdog();
        refreshFilesAfterTransfer();
      }
    }
    // Repeated snapshots must not extend the transfer deadline.
    if (transferBusy.value && (!isSnapshot || nativeWatchdog === undefined))
      armNativeWatchdog();
  }
  const syncFolder = () => {
    androidFolder.value = receiveFolder();
    folderReady.value = !android || android.hasReceiveFolder?.() !== false;
  };
  const resume = () => {
    syncFolder();
    android?.syncSession?.();
  };
  onMounted(() => {
    window.addEventListener("plumelink-download-recovery", nativeRecovery);
    window.addEventListener("plumelink-native-transfer", nativeTransfer);
    window.addEventListener("plumelink-native-folder", syncFolder);
    window.addEventListener("pageshow", resume);
    resume();
  });
  onUnmounted(() => {
    clearTimeout(recoveryTimer);
    window.removeEventListener("plumelink-download-recovery", nativeRecovery);
    filesRequestSerial++;
    filesAbort?.abort();
    clearSuccess();
    if (transferBusy.value && !android?.syncSession) cancelTransfer();
    clearNativeWatchdog();
    clearCancelWatchdog();
    window.removeEventListener("plumelink-native-transfer", nativeTransfer);
    window.removeEventListener("plumelink-native-folder", syncFolder);
    window.removeEventListener("pageshow", resume);
  });
  return {
    unfinishedDownloads, recoveryBusy, recoveryMessage, resolveUnfinishedDownload,
    sharedFiles,
    filesLoading,
    filesError,
    transferBusy,
    canStartTransfer,
    transferCancelling,
    transferProgress,
    transferMessage,
    transferHistory,
    androidFolder,
    folderReady,
    loadFiles,
    uploadFiles,
    chooseUploadFiles,
    downloadFile,
    cancelTransfer,
    clearTransferHistory,
  };
}
