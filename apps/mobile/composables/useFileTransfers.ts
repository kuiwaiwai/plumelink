import { locale, t } from "../../../shared/i18n";
import { ref, onMounted, onUnmounted, watch, type Ref } from "vue";
import { storage } from "../storage";
import { androidBridge } from "../native/bridge";
import { uploadResumable, MAX_FILE_BYTES, isSafeFileName } from "../file-transfer";
import type { useConnection } from "../connection";

type TransferConnection = Pick<
  ReturnType<typeof useConnection>,
  "api" | "token" | "latency" | "fileAccessPending"
>;
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
  const transferHistory = ref<string[]>([]);
  try {
    const history: unknown = JSON.parse(
      storage.getItem("plumelink-transfer-history") || "[]",
    );
    if (Array.isArray(history))
      transferHistory.value = history
        .filter((item): item is string => typeof item === "string")
        .slice(0, 10);
  } catch {}
  let transferAbort: AbortController | null = null;
  let transferCancelled = false;
  let filesRequestSerial = 0;
  let refreshAfterTransfer = false;
  let ignoreLateNativeEvents = false;
  let nativeTransferBlocked: "stale" | "reconnecting" | false = false;
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
      nativeTransferBlocked = "stale";
      transferMessage.value = t('文件传输长时间没有响应，已停止，请检查连接后重试');
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
      nativeTransferBlocked = "stale";
      transferMessage.value = t('取消传输没有收到电脑确认，请检查连接后重试');
      refreshFilesAfterTransfer();
    }, 15_000);
  };
  const receiveFolder = () => android?.getReceiveFolder() || t('浏览器默认下载目录');
  const androidFolder = ref(receiveFolder());
  const folderReady = ref(!android || android.hasReceiveFolder?.() !== false);
  watch(locale, () => { androidFolder.value = receiveFolder(); });
  function rememberTransfer(message: string) {
    transferHistory.value = [
      message,
      ...transferHistory.value.filter((item) => item !== message),
    ].slice(0, 10);
    try {
      storage.setItem(
        "plumelink-transfer-history",
        JSON.stringify(transferHistory.value),
      );
    } catch {}
  }
  function clearTransferHistory() {
    transferHistory.value = [];
    storage.removeItem("plumelink-transfer-history");
  }
  async function loadFiles() {
    const serial = ++filesRequestSerial;
    filesLoading.value = true;
    filesError.value = "";
    if (!canFiles.value) {
      sharedFiles.value = [];
      filesLoading.value = false;
      return;
    }
    try {
      const response = await remote.api("/api/files");
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
      if (serial === filesRequestSerial) filesLoading.value = false;
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
    if (files.length > 16) {
      transferMessage.value = t('每次最多选择 16 个文件');
      return;
    }
    const invalidName = files.find(file => !isSafeFileName(file.name));
    if (invalidName) {
      transferMessage.value = t('文件名无效，请重命名后重试');
      return;
    }
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
        transferProgress.value = 0;
        if (!Number.isSafeInteger(file.size) || file.size < 0 || file.size > MAX_FILE_BYTES)
          throw new Error(t('{0} 大小无效或超过 2 GB', file.name));
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
            transferProgress.value = file.size === 0 ? 100 : Math.floor((bytes / file.size) * 100);
            transferMessage.value = retrying
              ? t('网络中断，正在续传 {0}…', file.name)
              : t('正在发送 {0}{1}…', file.name, formatRate(bytes));
          },
        });
        showSuccess(t('{0} 已保存到电脑', name));
        rememberTransfer(t('发送：{0}', name));
      }
    } catch (e) {
      transferMessage.value = e instanceof Error ? e.message : t('文件发送失败');
    } finally {
      transferAbort = null;
      transferBusy.value = false;
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
    if (nativeTransferBlocked) {
      transferMessage.value = t('上一项传输仍在停止，请稍后重试');
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
    filesLoading.value = false;
    filesError.value = "";
    sharedFiles.value = [];
    clearSuccess();
    if (mayContinueNativeTransfer()) return;
    const wasTransferring = transferBusy.value;
    transferCancelled = true;
    transferAbort?.abort();
    if (android && !transferAbort && transferBusy.value) nativeTransferBlocked = "reconnecting";
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
    } : null;
    if (!detail) return;
    if (typeof detail.folder === "string" && detail.folder.length <= 512) {
      androidFolder.value = detail.folder;
      folderReady.value = !android || android.hasReceiveFolder?.() !== false;
    }
    if (!canFiles.value && !(transferBusy.value && mayContinueNativeTransfer())) return;
    const isSnapshot = detail.snapshot === true;
    if (nativeTransferBlocked === "stale") {
      if (detail.done !== true) return;
      nativeTransferBlocked = false;
      ignoreLateNativeEvents = false;
      // A stale completion clears the task guard but must leave the displayed error intact.
      refreshFilesAfterTransfer();
      return;
    }
    if (nativeTransferBlocked === "reconnecting") {
      if (!isSnapshot) return;
      nativeTransferBlocked = false;
      ignoreLateNativeEvents = false;
    }
    // Discard late progress and snapshots after cancellation or timeout.
    if (isSnapshot && ignoreLateNativeEvents) return;
    if (!isSnapshot && ignoreLateNativeEvents) return;
    if (isSnapshot) ignoreLateNativeEvents = false;
    // Repeated snapshots must not extend the transfer deadline.
    if (transferBusy.value && (!isSnapshot || nativeWatchdog === undefined))
      armNativeWatchdog();
    if (typeof detail.message === "string" && detail.message.length <= 512) {
      const succeeded = detail.done !== false && (detail.succeeded === true ||
        (detail.succeeded === undefined && (detail.message.includes('已保存') || detail.message.includes('已发送'))));
      if (succeeded) {
        if (!isSnapshot || transferBusy.value) {
          showSuccess(detail.message);
          rememberTransfer(detail.message);
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
    window.addEventListener("plumelink-native-transfer", nativeTransfer);
    window.addEventListener("plumelink-native-folder", syncFolder);
    window.addEventListener("pageshow", resume);
    resume();
  });
  onUnmounted(() => {
    clearSuccess();
    if (transferBusy.value && !android?.syncSession) cancelTransfer();
    clearNativeWatchdog();
    clearCancelWatchdog();
    window.removeEventListener("plumelink-native-transfer", nativeTransfer);
    window.removeEventListener("plumelink-native-folder", syncFolder);
    window.removeEventListener("pageshow", resume);
  });
  return {
    sharedFiles,
    filesLoading,
    filesError,
    transferBusy,
    transferCancelling,
    transferProgress,
    transferMessage,
    transferHistory,
    androidFolder,
    folderReady,
    loadFiles,
    uploadFiles,
    downloadFile,
    cancelTransfer,
    clearTransferHistory,
  };
}
