import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ref } from "vue";
import { readSharedSelection, useSharedFiles } from "../apps/mobile/composables/useSharedFiles";

const lifecycle = vi.hoisted(() => ({ destroy: [] as (() => void)[] }));
vi.mock("vue", async original => ({
  ...await original<typeof import("vue")>(),
  onMounted: (fn: () => void) => fn(),
  onUnmounted: (fn: () => void) => lifecycle.destroy.push(fn),
}));
const file = { id: "a".repeat(32), name: "notes.txt", size: 0, pending: false, error: "" };
const value = (revision = "b".repeat(32)) => ({ revision, files: [file], message: "" });
const native = { getSharedFiles: vi.fn(), sendSharedFiles: vi.fn(), clearSharedFiles: vi.fn(), removeSharedFile: vi.fn(), retrySharedFile: vi.fn() };
const update = (detail: unknown) => window.dispatchEvent(new CustomEvent("plumelink-shared-files", { detail }));

beforeEach(() => {
  vi.useFakeTimers(); vi.clearAllMocks(); lifecycle.destroy = [];
  vi.stubGlobal("window", new EventTarget()); vi.stubGlobal("PlumeLinkAndroid", native);
  native.getSharedFiles.mockReturnValue(JSON.stringify(value()));
});
afterEach(() => { lifecycle.destroy.forEach(fn => fn()); vi.useRealTimers(); vi.unstubAllGlobals(); });

it("receives cold-start and later shares without sending automatically", () => {
  const shares = useSharedFiles(ref(true));
  expect(shares.openRequest.value).toBe(1);
  update({ ...value("c".repeat(32)), open: true });
  expect(shares.openRequest.value).toBe(2);
  expect(native.sendSharedFiles).not.toHaveBeenCalled();
  expect(shares.selection.value.revision).toBe("c".repeat(32));
});
it("requires file access, metadata readiness and one confirmation at a time", () => {
  const allowed = ref(false), shares = useSharedFiles(allowed);
  shares.send(); expect(native.sendSharedFiles).not.toHaveBeenCalled();
  allowed.value = true;
  update({ ...value(), files: [{ ...file, pending: true }] });
  shares.send(); expect(native.sendSharedFiles).not.toHaveBeenCalled();
  update(value()); shares.send(); shares.send();
  expect(native.sendSharedFiles).toHaveBeenCalledExactlyOnceWith("b".repeat(32));
  update({ revision: "c".repeat(32), files: [], message: "" });
  expect(shares.canSend.value).toBe(false);
});
it("refreshes after an unanswered confirmation without retrying the upload", () => {
  const shares = useSharedFiles(ref(true)); shares.send();
  vi.advanceTimersByTime(8000);
  expect(native.sendSharedFiles).toHaveBeenCalledTimes(1);
  expect(native.getSharedFiles).toHaveBeenCalledTimes(2);
  expect(shares.submitting.value).toBe(false);
});
it("rejects malformed and unbounded snapshots instead of replacing valid pending files", () => {
  const shares = useSharedFiles(ref(true));
  for (const bad of [null, { ...value(), files: Array(17).fill(file) }, { ...value(), files: [file, file] },
    { ...value(), files: [{ ...file, size: Infinity }] }, { ...value(), files: [{ ...file, name: "bad\nname" }] }]) {
    expect(readSharedSelection(bad)).toBeUndefined(); update(bad);
  }
  expect(shares.selection.value.files).toEqual([file]);
  shares.remove(file);
  expect(native.removeSharedFile).toHaveBeenCalledWith("b".repeat(32), file.id);
  shares.clear(); expect(native.clearSharedFiles).toHaveBeenCalledWith("b".repeat(32));
});
it("ignores incoming events after the page is destroyed", () => {
  const shares = useSharedFiles(ref(true)); lifecycle.destroy.forEach(fn => fn());
  update({ ...value("d".repeat(32)), open: true });
  expect(shares.selection.value.revision).toBe("b".repeat(32));
});

it("retries failed metadata while offline without uploading, and waits for native acknowledgement", () => {
  const shares = useSharedFiles(ref(false));
  const failed = { ...file, error: "Provider busy" };
  update({ ...value(), files: [failed] });
  expect(shares.canRetry(failed)).toBe(true);
  shares.retry(failed); shares.retry(failed);
  expect(native.retrySharedFile).toHaveBeenCalledExactlyOnceWith("b".repeat(32), failed.id);
  expect(shares.submitting.value).toBe(true);
  const pending = { ...failed, id: "c".repeat(32), error: "", pending: true };
  update({ ...value("d".repeat(32)), files: [pending] });
  expect(shares.submitting.value).toBe(false);
  expect(shares.canRetry(failed)).toBe(false);
  expect(shares.canRetry(pending)).toBe(false);
  expect(native.sendSharedFiles).not.toHaveBeenCalled();
});

it("does not retry ready files or compete with another metadata read or send", () => {
  const shares = useSharedFiles(ref(true));
  shares.retry(file); expect(native.retrySharedFile).not.toHaveBeenCalled();
  const failed = { ...file, error: "Read failed" };
  update({ ...value(), files: [failed, { ...file, id: "c".repeat(32), pending: true }] });
  shares.retry(failed); expect(native.retrySharedFile).not.toHaveBeenCalled();
  update(value()); shares.send();
  shares.retry(failed); expect(native.retrySharedFile).not.toHaveBeenCalled();
});

it("refreshes an unanswered metadata retry without replaying it", () => {
  const shares = useSharedFiles(ref(false));
  const failed = { ...file, error: "Read failed" };
  update({ ...value(), files: [failed] });
  native.getSharedFiles.mockReturnValue(JSON.stringify({ ...value(), files: [failed] }));
  shares.retry(failed); vi.advanceTimersByTime(8000);
  expect(shares.submitting.value).toBe(false);
  expect(native.retrySharedFile).toHaveBeenCalledTimes(1);
  expect(native.sendSharedFiles).not.toHaveBeenCalled();
  expect(shares.canRetry(failed)).toBe(true);
});
