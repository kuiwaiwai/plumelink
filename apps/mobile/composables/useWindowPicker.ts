import { computed, onScopeDispose, ref, watch, type ComputedRef, type Ref } from "vue";
import type { RemoteWindow } from "../connection";
import type { Destination } from "../native/navigation";
import { displayMessage, locale, t } from "../../../shared/i18n";
import { errorText } from "../../../shared/format";

type Client = { listWindows(signal?: AbortSignal): Promise<RemoteWindow[]>; switchWindow(hwnd: number): Promise<void> };
type Snapshot = { windows: RemoteWindow[]; silent: boolean };

export function useWindowPicker(
  remote: Client, destination: Readonly<Ref<Destination>>, canInput: ComputedRef<boolean>, foreground: Readonly<Ref<boolean>>,
  notice: (message: string, duration?: number) => void, dismissFeedback: () => void,
  taskView: () => Promise<boolean>, backToRemote: () => void,
) {
  const windowList = ref<RemoteWindow[]>([]), windowLoading = ref(false), windowListReady = ref(false);
  const windowError = ref(""), windowSwitching = ref(0), openingTaskView = ref(false);
  const windowQuery = ref(""), windowSearchShown = ref(false);
  const visibleWindows = computed(() => {
    const normalize = (value: string) => value.normalize("NFKC").toLocaleLowerCase(locale.value);
    const words = normalize(windowQuery.value).trim().split(/\s+/).filter(Boolean);
    return windowList.value.filter(entry => {
      const title = normalize(entry.title), app = normalize(entry.app);
      return words.every(word => title.includes(word) || app.includes(word));
    });
  });
  const interacting = ref(false);
  const active = computed(() => destination.value === "windows" && canInput.value && foreground.value);
  let request: AbortController | undefined, silentRequest = false, pending: Snapshot | undefined;
  let generation = 0, disposed = false, retryDelay = 3000;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function cancelTimer() { clearTimeout(timer); timer = undefined; }
  function stopRead() { request?.abort(); request = undefined; pending = undefined; windowLoading.value = false; }
  function schedule() {
    cancelTimer();
    if (disposed || !active.value || request || pending || interacting.value || windowSwitching.value || openingTaskView.value || retryDelay < 0) return;
    timer = setTimeout(() => { timer = undefined; void readWindows(true); }, retryDelay);
  }
  function apply(snapshot: Snapshot) {
    const previous = new Map(windowList.value.map(entry => [entry.hwnd, entry]));
    const received = new Map(snapshot.windows.map(entry => [entry.hwnd, entry]));
    const ordered = snapshot.silent
      ? [...windowList.value.filter(entry => received.has(entry.hwnd)).map(entry => received.get(entry.hwnd)!),
        ...snapshot.windows.filter(entry => !previous.has(entry.hwnd))]
      : snapshot.windows;
    const next = ordered.map(entry => {
      const old = previous.get(entry.hwnd);
      if (!old) return entry;
      if (old.title !== entry.title || old.app !== entry.app || old.minimized !== entry.minimized || old.icon !== entry.icon) Object.assign(old, entry);
      return old;
    });
    if (next.length !== windowList.value.length || next.some((entry, index) => entry !== windowList.value[index])) windowList.value = next;
    windowError.value = ""; windowListReady.value = true; windowLoading.value = false; retryDelay = 3000;
  }
  async function readWindows(silent: boolean, clearFeedback = false) {
    if (disposed || !active.value || windowSwitching.value || openingTaskView.value) return;
    if (request) { if (silent || !silentRequest) return; stopRead(); }
    cancelTimer(); pending = undefined;
    const owner = new AbortController(); request = owner; silentRequest = silent;
    if (clearFeedback) dismissFeedback();
    if (!silent) { windowLoading.value = true; windowListReady.value = false; windowError.value = ""; }
    try {
      const windows = await remote.listWindows(owner.signal);
      if (request !== owner || !active.value) return;
      const snapshot = { windows, silent };
      if (interacting.value) pending = snapshot;
      else apply(snapshot);
    } catch (error) {
      if (request !== owner) return;
      windowError.value = errorText(error, t("无法获取窗口列表")); windowListReady.value = false;
      retryDelay = windowError.value.includes("窗口枚举不可用") ? -1 : 6000;
      if (!silent && windowList.value.length) notice(displayMessage(windowError.value), 6000);
    } finally {
      if (request === owner) {
        request = undefined;
        if (!pending || silent) windowLoading.value = false;
        schedule();
      }
    }
  }
  function loadWindows(clearFeedback = false) { return readWindows(false, clearFeedback); }
  function toggleWindowSearch() {
    windowSearchShown.value = !windowSearchShown.value;
    if (!windowSearchShown.value) windowQuery.value = "";
  }
  function setWindowInteracting(value: unknown) {
    if (typeof value !== "boolean" || !active.value) return;
    interacting.value = value;
    if (value) cancelTimer();
  }
  watch(interacting, value => {
    if (value || !active.value) return;
    if (pending && !windowSwitching.value && !openingTaskView.value) { const snapshot = pending; pending = undefined; apply(snapshot); }
    schedule();
  });
  async function switchWindow(entry: RemoteWindow) {
    if (!active.value || !windowListReady.value || openingTaskView.value || windowSwitching.value || !visibleWindows.value.some(item => item.hwnd === entry.hwnd)) return;
    const owner = generation; cancelTimer(); stopRead();
    windowSwitching.value = entry.hwnd; dismissFeedback(); windowError.value = "";
    let closed = false;
    try {
      await remote.switchWindow(entry.hwnd);
      if (owner !== generation) return;
      entry.minimized = false;
      const title = Array.from(entry.title);
      notice(t("已切换到 {0}", title.length > 48 ? title.slice(0, 47).join("") + "…" : entry.title));
    } catch (error) {
      if (owner !== generation) return;
      const message = errorText(error, t("切换窗口失败"));
      windowError.value = message === "系统拒绝切换到该窗口，请先在电脑上激活任意窗口后重试"
        ? "窗口尚未切到前台，请打开任务视图选择窗口" : message;
      notice(displayMessage(windowError.value), 6000); closed = message.includes("窗口已关闭");
    } finally {
      if (owner === generation) {
        windowSwitching.value = 0;
        if (active.value && (closed || !windowListReady.value)) void loadWindows(); else schedule();
      }
    }
  }
  async function openWindowTaskView() {
    if (!active.value || windowSwitching.value || openingTaskView.value) return;
    const owner = generation; cancelTimer(); stopRead(); openingTaskView.value = true;
    try { if (await taskView() && owner === generation) backToRemote(); }
    finally { if (owner === generation) { openingTaskView.value = false; if (active.value && !windowListReady.value) void loadWindows(); else schedule(); } }
  }
  watch([destination, canInput, foreground], ([page, allowed], previous) => {
    cancelTimer(); stopRead(); interacting.value = false; windowListReady.value = false;
    if (page !== previous[0] || allowed !== previous[1]) {
      generation++; windowList.value = []; windowError.value = "";
      windowSwitching.value = 0; openingTaskView.value = false; retryDelay = 3000;
    }
    if (page !== previous[0]) { windowQuery.value = ""; windowSearchShown.value = false; }
    if (active.value) void loadWindows();
  }, { immediate: true });
  onScopeDispose(() => { disposed = true; generation++; cancelTimer(); stopRead(); });
  return { windowList, visibleWindows, windowQuery, windowSearchShown, toggleWindowSearch, windowLoading, windowListReady, windowError, windowSwitching, openingTaskView, loadWindows, switchWindow, openWindowTaskView, setWindowInteracting };
}
