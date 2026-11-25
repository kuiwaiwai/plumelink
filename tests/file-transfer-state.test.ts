import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { computed, nextTick, ref } from "vue";
import { languagePreference } from "../shared/i18n";
import * as fileTransfer from "../apps/mobile/file-transfer";
import { downloadRecoveryLabel, downloadRecoveryStatus, useFileTransfers } from "../apps/mobile/composables/useFileTransfers";

const lifecycle = vi.hoisted(() => ({ destroy: [] as (() => void)[] }));
let receiveFolderSelected = true;
let receiveFolderLabel = "下载";
vi.mock("vue", async original => ({
  ...await original<typeof import("vue")>(),
  onMounted: (fn: () => void) => fn(),
  onUnmounted: (fn: () => void) => lifecycle.destroy.push(fn),
}));

const native = {
  getReceiveFolder: () => receiveFolderLabel,
  hasReceiveFolder: () => receiveFolderSelected,
  downloadFile: vi.fn(),
  chooseUploadFiles: vi.fn(),
  cancelFileTransfer: vi.fn(),
  syncSession: vi.fn(),
  refreshReceiveFolder: vi.fn(),
  resolveUnfinishedDownload: vi.fn(),
};
const remote = {
  api: vi.fn(),
  token: () => "a".repeat(64),
  latency: ref<number | null>(20),
  fileAccessPending: computed(() => false),
};

beforeEach(() => {
  lifecycle.destroy = [];
  receiveFolderSelected = true;
  receiveFolderLabel = "下载";
  vi.clearAllMocks();
  vi.stubGlobal("window", new EventTarget());
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: vi.fn(), removeItem: vi.fn() });
  vi.stubGlobal("PlumeLinkAndroid", native);
});

it("validates local unfinished files and keeps cleanup available without a PC connection", () => {
  const transfers = useFileTransfers(remote, ref(false), ref(false));
  const file = { id: "a".repeat(32), name: "notes.bin", size: 8, expected: 16, available: true, version: "b".repeat(64) };
  window.dispatchEvent(new CustomEvent("plumelink-download-recovery", { detail: { files: [file, file, { ...file, id: "../../other" }], message: "" } }));
  expect(transfers.unfinishedDownloads.value).toEqual([file]);
  transfers.resolveUnfinishedDownload(file, true);
  const request = native.resolveUnfinishedDownload.mock.calls[0][3];
  expect(native.resolveUnfinishedDownload).toHaveBeenCalledWith(file.id, file.version, true, request);
  expect(transfers.recoveryBusy.value).toBe(true);
  window.dispatchEvent(new CustomEvent("plumelink-download-recovery", { detail: { request, files: [], message: "" } }));
  expect(transfers.recoveryBusy.value).toBe(false);
  expect(transfers.unfinishedDownloads.value).toEqual([]);
});

it("does not replay timed out cleanup or let its late response release a newer request", () => {
  vi.useFakeTimers();
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  const file = { id: "a".repeat(32), name: "notes.bin", size: 8, expected: 16, available: true, version: "b".repeat(64) };
  transfers.resolveUnfinishedDownload(file, false);
  const first = native.resolveUnfinishedDownload.mock.calls[0][3];
  transfers.resolveUnfinishedDownload(file, false);
  expect(native.resolveUnfinishedDownload).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(15_000);
  expect(transfers.recoveryBusy.value).toBe(false);
  expect(native.resolveUnfinishedDownload).toHaveBeenCalledTimes(1);
  transfers.resolveUnfinishedDownload(file, true);
  const second = native.resolveUnfinishedDownload.mock.calls[1][3];
  expect(second).not.toBe(first);
  window.dispatchEvent(new CustomEvent("plumelink-download-recovery", { detail: { request: first, files: [], message: "old" } }));
  expect(transfers.recoveryBusy.value).toBe(true);
  window.dispatchEvent(new CustomEvent("plumelink-download-recovery", { detail: { request: second, files: [], message: "current" } }));
  expect(transfers.recoveryBusy.value).toBe(false);
  expect(transfers.recoveryMessage.value).toBe("current");
});

it("releases local cleanup controls if the Android call fails before dispatch", () => {
  native.resolveUnfinishedDownload.mockImplementationOnce(() => { throw new Error("native unavailable"); });
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  transfers.resolveUnfinishedDownload({ id: "a".repeat(32), name: "notes.bin", size: 8, expected: 16, available: true, version: "b".repeat(64) }, true);
  expect(transfers.recoveryBusy.value).toBe(false);
  expect(transfers.recoveryMessage.value).toBe("native unavailable");
});

it("requires a selected Android folder before starting a download and recovers after selection", () => {
  receiveFolderSelected = false;
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  expect(transfers.folderReady.value).toBe(false);
  transfers.downloadFile({ id: 7, name: "demo.txt", size: 10 });
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toContain("请先选择 Android 接收目录");
  expect(native.downloadFile).not.toHaveBeenCalled();
  receiveFolderSelected = true;
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { folder: "文档 / 下载", message: "" } }));
  expect(transfers.folderReady.value).toBe(true);
  transfers.downloadFile({ id: 7, name: "demo.txt", size: 10 });
  expect(native.downloadFile).toHaveBeenCalledTimes(1);
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { done: true, message: "已保存 demo.txt" } }));
  expect(transfers.transferBusy.value).toBe(false);
});

it("refreshes the Android folder label when returning from the system picker", () => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  expect(transfers.androidFolder.value).toBe("下载");
  receiveFolderLabel = "内部存储 / 文档 / 接收文件";
  window.dispatchEvent(new Event("pageshow"));
  expect(transfers.androidFolder.value).toBe(receiveFolderLabel);
  expect(native.syncSession).toHaveBeenCalled();
});

it("updates Android folder availability without restarting or ending a file transfer", () => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  transfers.downloadFile({ id: 7, name: "demo.txt", size: 10 });
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: false, progress: 37, message: "正在接收 demo.txt" },
  }));
  native.syncSession.mockClear();
  receiveFolderSelected = false;
  receiveFolderLabel = "接收目录不可用，请重新选择。";
  window.dispatchEvent(new Event("plumelink-native-folder"));
  expect(transfers.folderReady.value).toBe(false);
  expect(transfers.androidFolder.value).toBe(receiveFolderLabel);
  expect(transfers.transferBusy.value).toBe(true);
  expect(transfers.transferProgress.value).toBe(37);
  expect(native.syncSession).not.toHaveBeenCalled();
});

it("checks local folder availability on file refresh even without PC file permission", async () => {
  const transfers = useFileTransfers(remote, ref(false), ref(false));
  native.refreshReceiveFolder.mockClear();
  await transfers.loadFiles();
  expect(native.refreshReceiveFolder).toHaveBeenCalledOnce();
  receiveFolderSelected = false;
  receiveFolderLabel = "接收目录不可用，请重新选择。";
  window.dispatchEvent(new Event("plumelink-native-folder"));
  expect(transfers.folderReady.value).toBe(false);
  expect(transfers.androidFolder.value).toBe(receiveFolderLabel);
  expect(transfers.transferBusy.value).toBe(false);
});

it("refreshes the directory without resetting an active transfer or requesting a task snapshot", async () => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  transfers.downloadFile({ id: 7, name: "demo.txt", size: 10 });
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { done: false, progress: 37 } }));
  remote.api.mockResolvedValue(new Response(JSON.stringify({ files: [] })));
  native.refreshReceiveFolder.mockClear(); native.syncSession.mockClear();
  await transfers.loadFiles();
  expect(native.refreshReceiveFolder).toHaveBeenCalledOnce();
  expect(native.syncSession).not.toHaveBeenCalled();
  expect(transfers.transferBusy.value).toBe(true);
  expect(transfers.transferProgress.value).toBe(37);
});

it("updates a deeply nested Android folder immediately without interrupting transfer progress", () => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  transfers.downloadFile({ id: 7, name: "demo.txt", size: 10 });
  const folder = "Documents/项目资料/Project Notes/".repeat(24);
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { folder, done: false, progress: 37 },
  }));
  expect(transfers.androidFolder.value).toBe(folder);
  expect(transfers.folderReady.value).toBe(true);
  expect(transfers.transferBusy.value).toBe(true);
  expect(transfers.transferProgress.value).toBe(37);
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { folder: "x".repeat(65536) } }));
  expect(transfers.androidFolder.value).toBe(folder);
});
afterEach(() => {
  languagePreference.value = "system";
  lifecycle.destroy.forEach(fn => fn());
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("clamps native progress and only done=true ends a transfer", () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 7, name: "demo.txt", size: 10 });
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { progress: 150, done: false, message: "正在接收 demo.txt" } }));
  expect(transfers.transferProgress.value).toBe(100);
  expect(transfers.transferBusy.value).toBe(true);
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { progress: -4, done: true, message: "已保存 demo.txt" } }));
  expect(transfers.transferProgress.value).toBe(0);
  expect(transfers.transferBusy.value).toBe(false);
});

it("ignores malformed native transfer events", () => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  expect(() => window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: null }))).not.toThrow();
  expect(transfers.transferBusy.value).toBe(false);
});
it("does not let a non-boolean snapshot bypass late-event protection", () => {
  vi.useFakeTimers();
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 44, name: "late.bin", size: 10 });
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: false, message: "正在接收 late.bin", snapshot: "yes" },
  }));
  transfers.cancelTransfer();
  vi.advanceTimersByTime(15_000);
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: true, message: "late.bin 已保存", snapshot: "yes" },
  }));
  expect(transfers.transferMessage.value).toContain("取消尚未完成");
  vi.useRealTimers();
});
it("accepts an authoritative in-flight snapshot and keeps the transfer busy", () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 45, name: "restored.bin", size: 10 });
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: false, progress: 37, message: "正在接收 restored.bin", snapshot: true },
  }));
  expect(transfers.transferBusy.value).toBe(true);
  expect(transfers.transferProgress.value).toBe(37);
  expect(transfers.transferMessage.value).toBe("正在接收 restored.bin");
});

it.each([false, true])("times out a native upload first discovered by an event (snapshot=%s)", async snapshot => {
  vi.useFakeTimers();
  try {
    const transfers = useFileTransfers(remote, ref(true), ref(false));
    window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
      detail: { done: false, progress: 0, message: "正在发送 picked.bin", snapshot },
    }));
    expect(transfers.transferBusy.value).toBe(true);
    await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
    expect(native.cancelFileTransfer).toHaveBeenCalledOnce();
    expect(transfers.transferBusy.value).toBe(false);
    expect(transfers.canStartTransfer.value).toBe(false);
    expect(transfers.transferMessage.value).toContain("已请求停止");
  } finally { vi.useRealTimers(); }
});

it("accepts a completed snapshot and clears the native transfer state", () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 46, name: "restored-done.bin", size: 10 });
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: true, progress: 100, message: "已保存 restored-done.bin", snapshot: true },
  }));
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferProgress.value).toBe(100);
  expect(transfers.transferMessage.value).toBe("已保存 restored-done.bin");
});

it("times out a restored in-flight snapshot instead of hanging forever", async () => {
  vi.useFakeTimers();
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 47, name: "restored-timeout.bin", size: 10 });
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: false, progress: 12, message: "正在接收 restored-timeout.bin", snapshot: true },
  }));
  await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toContain("长时间没有响应");
  vi.useRealTimers();
});
it("filters malformed shared file metadata", async () => {
  const canFiles = ref(true);
  remote.api.mockResolvedValueOnce({ ok: true, json: async () => ({ files: [
    { id: 1, name: "ok.txt", size: 3 },
    { id: 1, name: "duplicate.txt", size: 3 },
    { id: 2, name: "bad\nname", size: 3 },
    { id: 4, name: ".", size: 0 },
    { id: 5, name: "..", size: 0 },
    { id: 6, name: "CON.txt", size: 0 },
    { id: 7, name: "trailing.", size: 0 },
    { id: 8, name: "trailing ", size: 0 },
    { id: 9, name: ".plumelink-temp", size: 0 },
    { id: 3, name: "too-big.bin", size: 2 ** 31 + 1 },
    { id: "4", name: "wrong-id", size: 3 },
    null,
  ] }) });
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  await transfers.loadFiles();
  expect(transfers.sharedFiles.value).toEqual([{ id: 1, name: "ok.txt", size: 3 }]);
});
it("rejects invalid upload names before starting a transfer", async () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  const input = { files: [{ name: "bad\nname.txt", size: 10 }], value: "" } as unknown as HTMLInputElement;
  await transfers.uploadFiles({ target: input } as unknown as Event);
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toBe("文件名无效，请重命名后重试");
});

it("keeps native transfer busy until cancellation is acknowledged", () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 12, name: "cancel.bin", size: 20 });
  transfers.cancelTransfer();
  expect(transfers.transferBusy.value).toBe(true);
  expect(transfers.transferMessage.value).toBe("正在取消传输…");
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: true, progress: 0, message: "已取消传输" },
  }));
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toBe("已取消传输");
});

it("ends a native cancellation after the acknowledgement watchdog expires", async () => {
  vi.useFakeTimers();
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 17, name: "cancel-timeout.bin", size: 20 });
  transfers.cancelTransfer();
  await vi.advanceTimersByTimeAsync(15_000);
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toContain("取消尚未完成");
  vi.useRealTimers();
});

it("preserves the timeout message when a late native completion arrives", async () => {
  vi.useFakeTimers();
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 19, name: "late-complete.bin", size: 20 });
  await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
  expect(transfers.transferMessage.value).toContain("长时间没有响应");
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: true, progress: 100, message: "late-complete.bin 已保存" },
  }));
  expect(transfers.transferMessage.value).toContain("长时间没有响应");
  vi.useRealTimers();
});

it("does not resurrect a timed-out transfer from a late snapshot", async () => {
  vi.useFakeTimers();
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 48, name: "stale-snapshot.bin", size: 20 });
  await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
  const timeoutMessage = transfers.transferMessage.value;
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: false, progress: 50, message: "正在接收 stale-snapshot.bin", snapshot: true },
  }));
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toBe(timeoutMessage);
  vi.useRealTimers();
});

it("blocks a new native download until the timed-out transfer confirms it stopped", async () => {
  vi.useFakeTimers();
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 49, name: "old.bin", size: 20 });
  await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
  transfers.downloadFile({ id: 50, name: "new.bin", size: 20 });
  expect(native.downloadFile).toHaveBeenCalledTimes(1);
  expect(transfers.transferMessage.value).toContain("上一项传输仍在停止");
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: true, progress: 0, message: "已取消传输" },
  }));
  transfers.downloadFile({ id: 50, name: "new.bin", size: 20 });
  expect(native.downloadFile).toHaveBeenCalledTimes(2);
  vi.useRealTimers();
});

it("blocks uploads too while native cancellation is still unconfirmed", async () => {
  vi.useFakeTimers();
  try {
    const upload = vi.spyOn(fileTransfer, "uploadResumable").mockResolvedValue("next.bin");
    const transfers = useFileTransfers(remote, ref(true), ref(false));
    transfers.downloadFile({ id: 54, name: "previous.bin", size: 20 });
    transfers.cancelTransfer();
    await vi.advanceTimersByTimeAsync(15_000);
    const input = { files: [{ name: "next.bin", size: 5 }], value: "" } as unknown as HTMLInputElement;
    await transfers.uploadFiles({ target: input } as unknown as Event);
    expect(upload).not.toHaveBeenCalled();
    expect(transfers.transferMessage.value).toContain("上一项传输仍在停止");
    window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
      detail: { done: true, succeeded: false, message: "已取消传输" },
    }));
    await transfers.uploadFiles({ target: input } as unknown as Event);
    expect(upload).toHaveBeenCalledTimes(1);
    expect(transfers.transferMessage.value).toBe("next.bin 已保存到电脑");
  } finally { vi.useRealTimers(); }
});

it("keeps the native picker disabled until a refreshed snapshot confirms the previous task ended", async () => {
  vi.useFakeTimers();
  try {
    const allowed = ref(true);
    const transfers = useFileTransfers(remote, allowed, ref(false));
    transfers.downloadFile({ id: 55, name: "stopping.bin", size: 20 });
    transfers.cancelTransfer();
    await vi.advanceTimersByTimeAsync(15_000);
    expect(transfers.transferBusy.value).toBe(false);
    expect(transfers.canStartTransfer.value).toBe(false);
    transfers.chooseUploadFiles();
    expect(native.chooseUploadFiles).not.toHaveBeenCalled();
    native.syncSession.mockClear();
    remote.api.mockResolvedValueOnce({ ok: true, json: async () => ({ files: [] }) });
    native.syncSession.mockImplementationOnce(() => window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
      detail: { done: true, snapshot: true, succeeded: false, message: "已取消传输" },
    })));
    await transfers.loadFiles();
    expect(native.syncSession).toHaveBeenCalledOnce();
    expect(transfers.canStartTransfer.value).toBe(true);
    transfers.chooseUploadFiles();
    expect(native.chooseUploadFiles).toHaveBeenCalledOnce();
    allowed.value = false;
    await nextTick();
    expect(transfers.canStartTransfer.value).toBe(false);
    transfers.chooseUploadFiles();
    expect(native.chooseUploadFiles).toHaveBeenCalledOnce();
  } finally { vi.useRealTimers(); }
});

it("clears the cancellation watchdog when file permission is revoked", async () => {
  vi.useFakeTimers();
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 18, name: "cancel-revoked.bin", size: 20 });
  transfers.cancelTransfer();
  canFiles.value = false;
  await nextTick();
  await vi.advanceTimersByTimeAsync(15_000);
  expect(transfers.transferMessage.value).toContain("权限已撤销");
  vi.useRealTimers();
});

it("revoking file permission cancels native transfer and clears the list", async () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 8, name: "demo.zip", size: 20 });
  canFiles.value = false;
  await nextTick();
  expect(native.cancelFileTransfer).toHaveBeenCalled();
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.sharedFiles.value).toEqual([]);
});

it("keeps a native transfer running while reconnecting, then cancels if permission is denied", async () => {
  const canFiles = ref(true);
  const fileAccessPending = ref(false);
  const transfers = useFileTransfers({ ...remote, fileAccessPending: computed(() => fileAccessPending.value) }, canFiles, ref(false));
  transfers.downloadFile({ id: 8, name: "resume.zip", size: 20 });
  fileAccessPending.value = true;
  canFiles.value = false;
  await nextTick();
  expect(native.cancelFileTransfer).not.toHaveBeenCalled();
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: false, progress: 50, message: "正在接收 resume.zip" },
  }));
  expect(transfers.transferBusy.value).toBe(true);
  expect(transfers.transferProgress.value).toBe(50);
  fileAccessPending.value = false;
  await nextTick();
  expect(native.cancelFileTransfer).toHaveBeenCalledOnce();
  expect(transfers.transferBusy.value).toBe(false);
});

it("clears stale list errors when file permission is revoked", async () => {
  const canFiles = ref(true);
  remote.api.mockRejectedValueOnce(new Error("电脑暂时不可用"));
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  await transfers.loadFiles();
  expect(transfers.filesError.value).toBe("电脑暂时不可用");
  canFiles.value = false;
  await nextTick();
  expect(transfers.filesError.value).toBe("");
});

it("ignores late in-progress native events after permission is revoked", async () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 14, name: "late.bin", size: 20 });
  canFiles.value = false;
  await nextTick();
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: false, progress: 50, message: "正在接收 late.bin" },
  }));
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toContain("权限已撤销");
});

it("ignores late completed native events after permission is revoked", async () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 16, name: "late-success.bin", size: 20 });
  canFiles.value = false;
  await nextTick();
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: true, progress: 100, message: "late-success.bin 已保存" },
  }));
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toContain("权限已撤销");
});

it("restores a native transfer after a temporary permission loss", async () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 51, name: "resume.bin", size: 20 });
  canFiles.value = false;
  await nextTick();
  canFiles.value = true;
  await nextTick();
  expect(native.syncSession).toHaveBeenCalled();
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: false, progress: 42, message: "正在接收 resume.bin", snapshot: true },
  }));
  expect(transfers.transferBusy.value).toBe(true);
  expect(transfers.transferProgress.value).toBe(42);
});

it("uses the restored native snapshot to release a reconnecting transfer guard", async () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 52, name: "reconnect.bin", size: 20 });
  canFiles.value = false;
  await nextTick();
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toContain("权限已撤销");

  canFiles.value = true;
  await nextTick();
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: true, progress: 100, message: "reconnect.bin 已保存", snapshot: true },
  }));
  expect(transfers.transferBusy.value).toBe(false);
  transfers.downloadFile({ id: 53, name: "after-reconnect.bin", size: 20 });
  expect(native.downloadFile).toHaveBeenCalledWith("/api/files/53", "after-reconnect.bin", "a".repeat(64));
});

it("clears a stale list error when a new transfer starts", async () => {
  const canFiles = ref(true);
  remote.api.mockRejectedValueOnce(new Error("电脑暂时不可用"));
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  await transfers.loadFiles();
  expect(transfers.filesError.value).toBe("电脑暂时不可用");
  transfers.downloadFile({ id: 15, name: "new.bin", size: 20 });
  expect(transfers.filesError.value).toBe("");
});

it("refreshing the shared file list preserves the latest transfer feedback", async () => {
  const canFiles = ref(true);
  remote.api.mockResolvedValue({
    ok: true,
    json: async () => ({ files: [] }),
  });
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 9, name: "demo.bin", size: 20 });
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { progress: 100, done: true, message: "demo.bin 已保存" },
  }));
  expect(transfers.transferMessage.value).toBe("demo.bin 已保存");
  await transfers.loadFiles();
  expect(transfers.transferMessage.value).toBe("demo.bin 已保存");
});

it("does not replace a completed transfer message when list refresh fails", async () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: 13, name: "done.bin", size: 20 });
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { done: true, progress: 100, message: "done.bin 已保存" },
  }));
  remote.api.mockRejectedValue(new Error("请求电脑超时，请检查连接后重试"));
  await transfers.loadFiles();
  expect(transfers.transferMessage.value).toBe("done.bin 已保存");
  expect(transfers.filesError.value).toBe("请求电脑超时，请检查连接后重试");
});

it("clears a list error after a later refresh succeeds", async () => {
  const canFiles = ref(true);
  remote.api.mockRejectedValueOnce(new Error("电脑暂时不可用"));
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  await transfers.loadFiles();
  expect(transfers.filesError.value).toBe("电脑暂时不可用");
  remote.api.mockResolvedValueOnce({ ok: true, json: async () => ({ files: [] }) });
  await transfers.loadFiles();
  expect(transfers.filesError.value).toBe("");
});


it("revoking permission ends an in-flight list loading state", async () => {
  const canFiles = ref(true);
  let resolveList!: (value: unknown) => void;
  remote.api.mockReturnValue(new Promise(resolve => { resolveList = resolve; }));
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  const pending = transfers.loadFiles();
  expect(transfers.filesLoading.value).toBe(true);
  canFiles.value = false;
  await nextTick();
  expect(transfers.filesLoading.value).toBe(false);
  resolveList({ ok: true, json: async () => ({ files: [] }) });
  await pending;
  expect(transfers.filesLoading.value).toBe(false);
});

it("rejects malformed download metadata before invoking the native bridge", () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  transfers.downloadFile({ id: -1, name: "..\\escape.bin", size: 20 });
  expect(native.downloadFile).not.toHaveBeenCalled();
  expect(transfers.transferMessage.value).toBe("文件信息无效，请刷新后重试");
});

it("ignores non-string native transfer labels", () => {
  const canFiles = ref(true);
  const transfers = useFileTransfers(remote, canFiles, ref(false));
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
    detail: { folder: { bad: true }, message: { bad: true }, progress: 25, done: false },
  }));
  expect(transfers.androidFolder.value).toBe("下载");
  expect(transfers.transferMessage.value).toBe("");
  expect(transfers.transferProgress.value).toBe(25);
});


it("expires successful feedback once, keeps history, and preserves later errors", async () => {
  vi.useFakeTimers();
  try {
    const canFiles = ref(true);
    const transfers = useFileTransfers(remote, canFiles, ref(false));
    const result = { done: true, succeeded: true, message: "demo.bin 已保存" };
    const emit = (detail: object) => window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail }));
    emit(result);
    expect(transfers.transferMessage.value).toBe(result.message);
    vi.advanceTimersByTime(2000);
    emit({ ...result, snapshot: true });
    vi.advanceTimersByTime(1000);
    expect(transfers.transferMessage.value).toBe("");
    emit({ ...result, snapshot: true });
    expect(transfers.transferMessage.value).toBe("");
    expect(transfers.transferHistory.value).toContain(result.message);
    emit(result);
    canFiles.value = false;
    await nextTick();
    expect(transfers.transferMessage.value).toBe("");
    canFiles.value = true;
    await nextTick();
    emit(result);
    emit({ done: true, succeeded: false, message: "接收失败" });
    vi.advanceTimersByTime(4000);
    expect(transfers.transferMessage.value).toBe("接收失败");
  } finally {
    vi.useRealTimers();
  }
});

it("validates the whole upload selection before sending any file", async () => {
  const send = vi.spyOn(fileTransfer, "uploadResumable");
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  const input = { files: [{ name: "first.txt", size: 10 }, { name: "oversize.bin", size: fileTransfer.MAX_FILE_BYTES + 1 }], value: "selected" };
  await transfers.uploadFiles({ target: input } as unknown as Event);
  expect(send).not.toHaveBeenCalled();
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferMessage.value).toContain("oversize.bin");
  expect(input.value).toBe("");
});

it("keeps batch progress across files and reports a partial failure", async () => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  const values: number[] = [];
  vi.spyOn(fileTransfer, "uploadResumable").mockImplementation(async (file, options) => {
    options.progress(0, false);
    values.push(transfers.transferProgress.value);
    options.progress(file.size, false);
    values.push(transfers.transferProgress.value);
    if (file.name === "second.txt") throw new Error("电脑没有响应");
    return file.name;
  });
  const input = { files: [{ name: "first.txt", size: 10 }, { name: "second.txt", size: 10 }], value: "" };
  await transfers.uploadFiles({ target: input } as unknown as Event);
  expect(values).toEqual([0, 50, 50, 99]);
  expect(transfers.transferMessage.value).toBe("已发送 1/2 个文件。电脑没有响应");
  expect(transfers.transferHistory.value).toEqual(["发送：first.txt"]);
  expect(transfers.transferBusy.value).toBe(false);
});

it("clears cancellation and permits another upload after a batch is canceled", async () => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  vi.spyOn(fileTransfer, "uploadResumable").mockImplementationOnce(async (_file, options) => {
    transfers.cancelTransfer();
    expect(options.signal.aborted).toBe(true);
    throw new Error("已取消传输");
  }).mockResolvedValueOnce("saved.txt");
  const event = { target: { files: [{ name: "retry.txt", size: 10 }], value: "" } } as unknown as Event;
  await transfers.uploadFiles(event);
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferCancelling.value).toBe(false);
  await transfers.uploadFiles(event);
  expect(transfers.transferMessage.value).toBe("saved.txt 已保存到电脑");
  expect(transfers.transferProgress.value).toBe(100);
});

it.each([1, 2])("keeps %i confirmed native uploads in history when the rest of a batch fails", completed => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  const message = `已发送 ${completed}/3 个文件。已取消传输`;
  window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: {
    done: true, succeeded: false, direction: "send", fileName: "confirmed.txt", fileCount: completed,
    message, progress: Math.floor(completed * 100 / 3),
  } }));
  expect(transfers.transferMessage.value).toBe(message);
  expect(transfers.transferBusy.value).toBe(false);
  expect(transfers.transferHistory.value).toEqual([completed === 1 ? "发送：confirmed.txt" : "已发送 2 个文件"]);
});

it("does not record unconfirmed native batch files as sent", () => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  for (const fileCount of [0, -1, 1.5, 17]) window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: {
    done: true, succeeded: false, direction: "send", fileName: "unsent.txt", fileCount, message: "发送失败",
  } }));
  expect(transfers.transferHistory.value).toEqual([]);
});

it("aborts superseded list requests and ignores a late old response", async () => {
  const transfers = useFileTransfers(remote, ref(true), ref(false));
  let first!: (value: unknown) => void;
  let oldSignal: AbortSignal | undefined;
  remote.api.mockImplementationOnce((_path, init) => {
    oldSignal = init.signal;
    return new Promise(resolve => { first = resolve; });
  }).mockResolvedValueOnce({ ok: true, json: async () => ({ files: [{ id: 2, name: "new.txt", size: 1 }] }) });
  const pending = transfers.loadFiles();
  await transfers.loadFiles();
  expect(oldSignal?.aborted).toBe(true);
  first({ ok: true, json: async () => ({ files: [{ id: 1, name: "old.txt", size: 1 }] }) });
  await pending;
  expect(transfers.sharedFiles.value.map(f => f.name)).toEqual(["new.txt"]);
  expect(transfers.filesLoading.value).toBe(false);
});
it("aborts list requests when file access is revoked", async () => {
  const allowed = ref(true);
  const transfers = useFileTransfers(remote, allowed, ref(false));
  let signal!: AbortSignal;
  remote.api.mockImplementationOnce((_path, init) => new Promise((_resolve, reject) => {
    signal = init.signal;
    signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
  }));
  const pending = transfers.loadFiles();
  allowed.value = false;
  await nextTick();
  await pending;
  expect(signal.aborted).toBe(true);
  expect(transfers.filesError.value).toBe("");
  expect(transfers.filesLoading.value).toBe(false);
});

it("preserves pending publication receipts and rejects malformed uncertainty flags", () => {
  const transfers = useFileTransfers(remote, ref(false), ref(false));
  const file = { id: "a".repeat(32), name: "notes.bin", size: 16, expected: 16, available: true, version: "b".repeat(64), pendingCommit: true };
  window.dispatchEvent(new CustomEvent("plumelink-download-recovery", { detail: { files: [
    file,
    { ...file, id: "c".repeat(32), pendingCommit: "true" },
    { ...file, id: "d".repeat(32), pendingCommit: null },
    { ...file, id: "e".repeat(32), pendingCommit: 1 },
    { ...file, id: "f".repeat(32), pendingCommit: false },
  ] } }));
  expect(transfers.unfinishedDownloads.value.map(entry => [entry.id, entry.pendingCommit])).toEqual([[file.id, true], ["f".repeat(32), false]]);
  const pending = transfers.unfinishedDownloads.value[0];
  expect(downloadRecoveryStatus(pending)).toBe("保存结果待确认");
  expect(downloadRecoveryLabel(transfers.unfinishedDownloads.value)).toBe("2 个待核对文件");
  transfers.resolveUnfinishedDownload(pending, false);
  const request = native.resolveUnfinishedDownload.mock.calls[0][3];
  expect(native.resolveUnfinishedDownload).toHaveBeenCalledWith(file.id, file.version, false, request);
  window.dispatchEvent(new CustomEvent("plumelink-download-recovery", { detail: { request, files: [], message: "" } }));
  expect(transfers.recoveryBusy.value).toBe(false);
});

it.each([
  ["zh-CN", "保存结果待确认", "1 个待核对文件"],
  ["zh-TW", "儲存結果待確認", "1 個待核對檔案"],
  ["en", "Save result not confirmed", "Files to review: 1"],
] as const)("keeps full-size but unconfirmed files distinct in %s", (language, status, label) => {
  languagePreference.value = language;
  const file = { id: "a".repeat(32), name: "notes.bin", size: 16, expected: 16, available: true, version: "b".repeat(64), pendingCommit: true };
  expect(downloadRecoveryStatus(file)).toBe(status);
  expect(downloadRecoveryLabel([file])).toBe(label);
  expect(downloadRecoveryStatus({ ...file, pendingCommit: false })).toContain("16 B / 16 B");
  expect(downloadRecoveryStatus({ ...file, available: false })).toBe(status);
});
