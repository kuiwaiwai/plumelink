import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { computed, effectScope, nextTick, ref, type EffectScope } from "vue";
import type { RemoteWindow } from "../apps/mobile/connection";
import type { Destination } from "../apps/mobile/native/navigation";
import { useWindowPicker } from "../apps/mobile/composables/useWindowPicker";
import { languagePreference } from "../shared/i18n";

let scope: EffectScope;
const entry = (hwnd: number, title = "Window " + hwnd): RemoteWindow => ({ hwnd, title, app: "Editor", minimized: false, icon: null });
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => { vi.useFakeTimers(); scope = effectScope(); languagePreference.value = "zh-CN"; });
afterEach(() => { scope.stop(); vi.useRealTimers(); languagePreference.value = "system"; });
const flush = async () => { await nextTick(); await vi.advanceTimersByTimeAsync(0); };
async function setup() {
  const destination = ref<Destination>("windows"), allowed = ref(true), foreground = ref(true);
  let catalog = [entry(1), entry(2)];
  const listWindows = vi.fn<(signal?: AbortSignal) => Promise<RemoteWindow[]>>().mockImplementation(async () => structuredClone(catalog));
  const switchWindow = vi.fn<(hwnd: number) => Promise<void>>().mockResolvedValue(undefined);
  const notice = vi.fn(), dismiss = vi.fn(), taskView = vi.fn<() => Promise<boolean>>().mockResolvedValue(true), back = vi.fn();
  const model = scope.run(() => useWindowPicker({ listWindows, switchWindow }, destination, computed(() => allowed.value), foreground, notice, dismiss, taskView, back))!;
  await flush();
  return { model, destination, allowed, foreground, listWindows, switchWindow, notice, dismiss, taskView, back, catalog: (value: RemoteWindow[]) => { catalog = value; } };
}
it("silently updates changed rows, removes closed windows and appends new windows without MRU jumps", async () => {
  const { model, catalog, listWindows } = await setup(), first = model.windowList.value[0];
  catalog([entry(3), { ...entry(1, "Updated"), minimized: true }]);
  await vi.advanceTimersByTimeAsync(3000);
  expect(listWindows).toHaveBeenCalledTimes(2);
  expect(model.windowList.value.map(row => row.hwnd)).toEqual([1, 3]);
  expect(model.windowList.value[0]).toBe(first);
  expect(first.title).toBe("Updated"); expect(first.minimized).toBe(true);
  expect(model.windowListReady.value).toBe(true); expect(model.windowLoading.value).toBe(false);
  const stable = model.windowList.value;
  await vi.advanceTimersByTimeAsync(3000);
  expect(model.windowList.value).toBe(stable);
});
it("manual refresh adopts the current MRU order and suspends selection until it finishes", async () => {
  const { model, catalog, listWindows, dismiss } = await setup(), response = deferred<RemoteWindow[]>();
  catalog([entry(2), entry(1)]); listWindows.mockReturnValueOnce(response.promise);
  const read = model.loadWindows(true);
  expect(model.windowListReady.value).toBe(false); expect(model.windowLoading.value).toBe(true); expect(dismiss).toHaveBeenCalledOnce();
  await model.switchWindow(model.windowList.value[0]);
  response.resolve([entry(2), entry(1)]); await read;
  expect(model.windowList.value.map(row => row.hwnd)).toEqual([2, 1]);
  expect(model.windowListReady.value).toBe(true); expect(model.windowLoading.value).toBe(false);
});
it("buffers an in-flight snapshot until the native scroll or touch has settled", async () => {
  const { model, listWindows } = await setup(), response = deferred<RemoteWindow[]>();
  listWindows.mockReturnValueOnce(response.promise); await vi.advanceTimersByTimeAsync(3000);
  expect(model.windowLoading.value).toBe(false); expect(model.windowListReady.value).toBe(true);
  model.setWindowInteracting(true); response.resolve([entry(2, "Changed"), entry(3)]); await flush();
  expect(model.windowList.value.map(row => row.hwnd)).toEqual([1, 2]);
  await vi.advanceTimersByTimeAsync(12000); expect(listWindows).toHaveBeenCalledTimes(2);
  model.setWindowInteracting(false); await flush();
  expect(model.windowList.value.map(row => row.hwnd)).toEqual([2, 3]);
  expect(model.windowList.value[0].title).toBe("Changed");
  await vi.advanceTimersByTimeAsync(2999); expect(listWindows).toHaveBeenCalledTimes(2);
  await vi.advanceTimersByTimeAsync(1); expect(listWindows).toHaveBeenCalledTimes(3);
});
it("does not begin a read while a finger is held down", async () => {
  const { model, listWindows } = await setup();
  model.setWindowInteracting(true); await vi.advanceTimersByTimeAsync(20000);
  expect(listWindows).toHaveBeenCalledTimes(1);
  model.setWindowInteracting(false); await flush(); await vi.advanceTimersByTimeAsync(3000);
  expect(listWindows).toHaveBeenCalledTimes(2);
});
it("keeps an explicit refresh loading until its buffered result can be safely applied", async () => {
  const { model, listWindows } = await setup(), response = deferred<RemoteWindow[]>();
  model.setWindowInteracting(true); listWindows.mockReturnValueOnce(response.promise);
  const operation = model.loadWindows(); response.resolve([entry(3)]); await operation;
  expect(model.windowLoading.value).toBe(true); expect(model.windowListReady.value).toBe(false);
  expect(model.windowList.value.map(row => row.hwnd)).toEqual([1, 2]);
  model.setWindowInteracting(false); await flush();
  expect(model.windowLoading.value).toBe(false); expect(model.windowListReady.value).toBe(true);
  expect(model.windowList.value.map(row => row.hwnd)).toEqual([3]);
});
it("does not overlap reads and lets explicit refresh supersede an old silent response", async () => {
  const { model, listWindows } = await setup(), response = deferred<RemoteWindow[]>();
  listWindows.mockReturnValueOnce(response.promise); await vi.advanceTimersByTimeAsync(3000);
  const signal = listWindows.mock.calls[1][0];
  await vi.advanceTimersByTimeAsync(15000); expect(listWindows).toHaveBeenCalledTimes(2);
  await model.loadWindows(); expect(signal?.aborted).toBe(true);
  response.resolve([entry(99)]); await flush();
  expect(model.windowList.value.map(row => row.hwnd)).toEqual([1, 2]);
});
it("stops polling behind another page or overlay and rejects old responses after resuming", async () => {
  const { model, foreground, destination, listWindows, allowed } = await setup(), response = deferred<RemoteWindow[]>();
  listWindows.mockReturnValueOnce(response.promise); await vi.advanceTimersByTimeAsync(3000);
  foreground.value = false; await flush(); expect(listWindows.mock.calls[1][0]?.aborted).toBe(true);
  response.resolve([entry(99)]); await flush(); await vi.advanceTimersByTimeAsync(9000);
  expect(listWindows).toHaveBeenCalledTimes(2); expect(model.windowListReady.value).toBe(false);
  foreground.value = true; await flush(); expect(model.windowList.value.map(row => row.hwnd)).toEqual([1, 2]);
  destination.value = "remote"; await flush(); await vi.advanceTimersByTimeAsync(9000); expect(listWindows).toHaveBeenCalledTimes(3);
  destination.value = "windows"; await flush(); expect(listWindows).toHaveBeenCalledTimes(4);
  allowed.value = false; await flush(); await vi.advanceTimersByTimeAsync(9000);
  expect(model.windowList.value).toHaveLength(0); expect(listWindows).toHaveBeenCalledTimes(4);
  allowed.value = true; await flush(); expect(listWindows).toHaveBeenCalledTimes(5);
});
it("makes one initial read when navigation and overlay dismissal happen together", async () => {
  const { model, destination, foreground, listWindows } = await setup();
  destination.value = "remote"; foreground.value = false; await flush();
  foreground.value = true; destination.value = "windows"; await flush();
  expect(listWindows).toHaveBeenCalledTimes(2);
  expect(model.windowListReady.value).toBe(true);
});
it("disables stale rows on a failed background read and retries without repeated notices", async () => {
  const { model, listWindows, switchWindow, notice } = await setup();
  listWindows.mockRejectedValueOnce(new Error("暂时无法连接电脑")); await vi.advanceTimersByTimeAsync(3000);
  expect(model.windowListReady.value).toBe(false); expect(model.windowList.value).toHaveLength(2);
  expect(model.windowError.value).toBe("暂时无法连接电脑"); expect(notice).not.toHaveBeenCalled();
  await model.switchWindow(model.windowList.value[0]); expect(switchWindow).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(5999); expect(listWindows).toHaveBeenCalledTimes(2);
  await vi.advanceTimersByTimeAsync(1); expect(listWindows).toHaveBeenCalledTimes(3); expect(model.windowListReady.value).toBe(true);
});
it("does not repeatedly request a feature the connected desktop does not support", async () => {
  const { model, listWindows } = await setup();
  listWindows.mockRejectedValueOnce(new Error("窗口枚举不可用")); await model.loadWindows();
  await vi.advanceTimersByTimeAsync(60000); expect(listWindows).toHaveBeenCalledTimes(2);
  await model.loadWindows(); expect(model.windowListReady.value).toBe(true);
});
it("a switch cancels an older refresh so it cannot restore a stale minimized state", async () => {
  const { model, listWindows, switchWindow } = await setup(), response = deferred<RemoteWindow[]>(), switching = deferred<void>();
  model.windowList.value[0].minimized = true; listWindows.mockReturnValueOnce(response.promise);
  await vi.advanceTimersByTimeAsync(3000); switchWindow.mockReturnValueOnce(switching.promise);
  const first = model.switchWindow(model.windowList.value[0]);
  expect(listWindows.mock.calls[1][0]?.aborted).toBe(true);
  await model.switchWindow(model.windowList.value[1]); expect(switchWindow).toHaveBeenCalledTimes(1);
  response.resolve([{ ...entry(1), minimized: true }]); await flush(); await vi.advanceTimersByTimeAsync(9000);
  expect(listWindows).toHaveBeenCalledTimes(2); expect(model.windowList.value).toHaveLength(2);
  switching.resolve(); await first; expect(model.windowList.value[0].minimized).toBe(false);
});
it("refreshes after a closed-window error and ignores a detached row", async () => {
  const { model, catalog, switchWindow, listWindows } = await setup(), removed = model.windowList.value[0];
  switchWindow.mockRejectedValueOnce(new Error("窗口已关闭")); catalog([entry(2)]);
  await model.switchWindow(removed); await flush(); expect(listWindows).toHaveBeenCalledTimes(2);
  expect(model.windowList.value.map(row => row.hwnd)).toEqual([2]);
  await model.switchWindow(removed); expect(switchWindow).toHaveBeenCalledTimes(1);
});
it("a completed switch on a previous page cannot overwrite a newly opened picker", async () => {
  const { model, destination, switchWindow, notice } = await setup(), response = deferred<void>();
  switchWindow.mockReturnValueOnce(response.promise); const operation = model.switchWindow(model.windowList.value[0]);
  destination.value = "remote"; await flush(); destination.value = "windows"; await flush();
  response.resolve(); await operation;
  expect(model.windowSwitching.value).toBe(0); expect(notice).not.toHaveBeenCalled(); expect(model.windowListReady.value).toBe(true);
});
it("keeps a backgrounded switch owned until its reply and refreshes on return", async () => {
  const { model, foreground, switchWindow, listWindows } = await setup(), response = deferred<void>();
  switchWindow.mockReturnValueOnce(response.promise); const operation = model.switchWindow(model.windowList.value[0]);
  foreground.value = false; await flush(); foreground.value = true; await flush();
  await model.switchWindow(model.windowList.value[1]); expect(switchWindow).toHaveBeenCalledTimes(1);
  expect(listWindows).toHaveBeenCalledTimes(1);
  response.resolve(); await operation; await flush(); expect(listWindows).toHaveBeenCalledTimes(2);
});
it("pauses polling while opening Task View, then navigates only after success", async () => {
  const { model, listWindows, taskView, back } = await setup(), response = deferred<boolean>();
  taskView.mockReturnValueOnce(response.promise); const operation = model.openWindowTaskView();
  await model.openWindowTaskView(); await vi.advanceTimersByTimeAsync(9000);
  expect(taskView).toHaveBeenCalledTimes(1); expect(listWindows).toHaveBeenCalledTimes(1); expect(back).not.toHaveBeenCalled();
  response.resolve(true); await operation; expect(back).toHaveBeenCalledOnce();
});
it("resumes refresh after Task View fails and clears timers when the page scope is destroyed", async () => {
  const { model, listWindows, taskView, back } = await setup();
  taskView.mockResolvedValueOnce(false); await model.openWindowTaskView(); expect(back).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(3000); expect(listWindows).toHaveBeenCalledTimes(2);
  scope.stop(); await vi.advanceTimersByTimeAsync(30000); expect(listWindows).toHaveBeenCalledTimes(2); expect(vi.getTimerCount()).toBe(0);
});
it("searches titles and app names together with width, case and whitespace normalization", async () => {
  const { model, catalog, listWindows } = await setup();
  catalog([{ ...entry(1, "Budget 2026"), app: "Excel.EXE" }, { ...entry(2, "年度報告"), app: "文字編輯器" }, entry(3, "Other")]);
  await model.loadWindows(); const reads = listWindows.mock.calls.length;
  for (const language of ["zh-CN", "zh-TW", "en"] as const) {
    languagePreference.value = language;
    model.windowQuery.value = "  ｂＵＤＧＥＴ　EXCEL  "; expect(model.visibleWindows.value.map(row => row.hwnd)).toEqual([1]);
    model.windowQuery.value = "報告 編輯"; expect(model.visibleWindows.value.map(row => row.hwnd)).toEqual([2]);
    model.windowQuery.value = "2026 missing"; expect(model.visibleWindows.value).toHaveLength(0);
    model.windowQuery.value = " \t "; expect(model.visibleWindows.value).toHaveLength(3);
  }
  expect(model.windowList.value).toHaveLength(3); expect(listWindows).toHaveBeenCalledTimes(reads);
});
it("keeps a live search current when titles change or matching windows open and close", async () => {
  const { model, catalog } = await setup();
  model.windowQuery.value = "report"; expect(model.visibleWindows.value).toHaveLength(0);
  catalog([entry(3, "Report new"), entry(1, "Report original"), entry(2)]);
  await vi.advanceTimersByTimeAsync(3000); expect(model.visibleWindows.value.map(row => row.hwnd)).toEqual([1, 3]);
  catalog([entry(1, "Renamed document"), entry(4, "Report another")]);
  await vi.advanceTimersByTimeAsync(3000); expect(model.visibleWindows.value.map(row => row.hwnd)).toEqual([4]);
  expect(model.windowQuery.value).toBe("report");
});
it("ignores a stale tap on a row hidden by a newer search", async () => {
  const { model, switchWindow } = await setup(), hidden = model.windowList.value[0];
  model.windowQuery.value = "Window 2";
  await model.switchWindow(hidden); expect(switchWindow).not.toHaveBeenCalled();
  await model.switchWindow(model.visibleWindows.value[0]); expect(switchWindow).toHaveBeenCalledExactlyOnceWith(2);
});
it("clears the filter when search closes or the user leaves the page, while retaining it over reconnect", async () => {
  const { model, destination, foreground, allowed } = await setup();
  model.toggleWindowSearch(); model.windowQuery.value = "Window 2";
  foreground.value = false; await flush(); foreground.value = true; await flush();
  allowed.value = false; await flush(); allowed.value = true; await flush();
  expect(model.windowSearchShown.value).toBe(true); expect(model.windowQuery.value).toBe("Window 2");
  expect(model.visibleWindows.value.map(row => row.hwnd)).toEqual([2]);
  model.toggleWindowSearch(); expect(model.windowQuery.value).toBe(""); expect(model.visibleWindows.value).toHaveLength(2);
  model.toggleWindowSearch(); model.windowQuery.value = "Window 2";
  destination.value = "remote"; await flush(); destination.value = "windows"; await flush();
  expect(model.windowSearchShown.value).toBe(false); expect(model.windowQuery.value).toBe("");
});
