import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { reactive, nextTick } from "vue";
import { languagePreference, t } from "../shared/i18n";
import { useMicrophone } from "../apps/mobile/composables/useMicrophone";
const lifecycle = vi.hoisted(() => ({ destroy: [] as (() => void)[] }));
vi.mock("vue", async original => ({
  ...await original<typeof import("vue")>(),
  onMounted: (fn: () => void) => fn(),
  onUnmounted: (fn: () => void) => lifecycle.destroy.push(fn),
}));
let win: EventTarget;
const native = { startMicrophone: vi.fn(), stopMicrophone: vi.fn(), lockMicrophone: vi.fn(), syncSession: vi.fn(), hasMicrophonePermission: vi.fn(() => true) };
beforeEach(() => {
  vi.useFakeTimers(); vi.clearAllMocks(); lifecycle.destroy = [];
  native.hasMicrophonePermission.mockReturnValue(true);
  win = new EventTarget();
  vi.stubGlobal("window", win);
  vi.stubGlobal("document", Object.assign(new EventTarget(), { hidden: false }));
  vi.stubGlobal("PlumeLinkAndroid", native);
  vi.stubGlobal("requestAnimationFrame", (fn: () => void) => setTimeout(fn, 16));
  vi.stubGlobal("cancelAnimationFrame", clearTimeout);
});
afterEach(() => { lifecycle.destroy.forEach(fn => fn()); vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals(); });
function state(phase: string, locked = true, message = "") {
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: 42, phase, locked, snapshot: true, began: Date.now() - 5000, level: message ? 0 : .3, message } }));
}
it("restores a locked capture through reload, background and unmount without stopping it", () => {
  const mic = useMicrophone({ allowed: false, enabled: false, activePage: true }, vi.fn());
  state("live"); expect(mic.mode.value).toBe("locked"); expect(mic.duration.value).toBe("00:05");
  win.dispatchEvent(new Event("pagehide"));
  lifecycle.destroy.forEach(fn => fn()); lifecycle.destroy = [];
  expect(native.stopMicrophone).not.toHaveBeenCalled();
});
it.each(["zh-CN", "zh-TW", "en"] as const)("shows the PC configuration path for an unselected audio device in %s", locale => {
  const previous = languagePreference.value;
  try {
    languagePreference.value = locale;
    const notice = vi.fn();
    useMicrophone({ allowed: true, enabled: true, activePage: true }, notice);
    state("live");
    const error = (message: string) => win.dispatchEvent(new CustomEvent("plumelink-microphone", {
      detail: { id: 42, phase: "error", locked: false, message },
    }));
    error("请在电脑端选择虚拟音频设备");
    const [message, duration] = notice.mock.calls[0];
    expect(message).toContain(t("偏好设置") + " → " + t("手机麦克风"));
    expect(message).toContain(t("音频送往"));
    expect(message).toContain(t("保存音频设备"));
    expect(duration).toBe(10000);
    error("电脑音频连接无响应，请检查局域网连接");
    expect(notice).toHaveBeenLastCalledWith("电脑音频连接无响应，请检查局域网连接");
  } finally {
    languagePreference.value = previous;
  }
});
it("ignores malformed native microphone events", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  expect(() => win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: null }))).not.toThrow();
  expect(mic.mode.value).toBe("idle");
});
it("normalizes invalid level and timing values from native events", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: 42, phase: "live", locked: true, snapshot: true, level: NaN, began: NaN, message: "" } }));
  expect(mic.level.value).toBe(0);
  expect(mic.duration.value).toMatch(/^\d{2}:\d{2}$/);
});
it("ignores unknown microphone phases", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: 42, phase: "future-state", locked: true, level: 1 } }));
  expect(mic.phase.value).toBe("idle");
  expect(mic.mode.value).toBe("idle");
});
it("cancels a pending stop gesture without ending locked capture", async () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  state("live");
  mic.keyboard({ key: "Enter", repeat: false, preventDefault() {} } as KeyboardEvent, true);
  await vi.advanceTimersByTimeAsync(100);
  mic.up();
  expect(mic.mode.value).toBe("locked");
  expect(mic.recording.value).toBe(true);
  expect(mic.intent.value).toBe(null);
  expect(mic.progress.value).toBe(0);
  expect(native.stopMicrophone).not.toHaveBeenCalled();
});
it("ignores another pointer loss but stops an unlocked capture on cancellation", async () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  mic.keyboard({ key: "Enter", repeat: false, preventDefault() {} } as KeyboardEvent, true);
  await vi.advanceTimersByTimeAsync(240);
  const id = native.startMicrophone.mock.lastCall![0];
  mic.up({ pointerId: 2 } as PointerEvent);
  expect(mic.mode.value).toBe("holding");
  expect(native.stopMicrophone).not.toHaveBeenCalled();
  mic.up();
  expect(mic.mode.value).toBe("idle");
  expect(native.stopMicrophone).toHaveBeenCalledWith(id);
});
it("meter snapshots do not reset the two-second stop gesture", async () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn()); state("live");
  mic.keyboard({ key: " ", repeat: false, preventDefault() {} } as KeyboardEvent, true);
  for (let i = 0; i < 21; i++) { await vi.advanceTimersByTimeAsync(100); state("live"); }
  expect(native.stopMicrophone).toHaveBeenCalledWith(42);
});
it("a notification stop is reflected after returning, and revoked permission stops capture", async () => {
  const props = reactive({ allowed: true, enabled: true, activePage: true });
  const mic = useMicrophone(props, vi.fn()); state("live"); state("idle", false);
  expect(mic.mode.value).toBe("idle"); expect(native.stopMicrophone).not.toHaveBeenCalled();
  mic.keyboard({ key: "Enter", repeat: false, preventDefault() {} } as KeyboardEvent, true);
  await vi.advanceTimersByTimeAsync(240);
  const current = native.startMicrophone.mock.lastCall![0];
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: current, phase: "live", level: .5 } }));
  props.allowed = false; await nextTick();
  expect(mic.mode.value).toBe("idle"); expect(native.stopMicrophone).toHaveBeenCalledExactlyOnceWith(current);
});
it("stops a capture when the connection disappears", async () => {
  const props = reactive({ allowed: true, enabled: true, activePage: true });
  const mic = useMicrophone(props, vi.fn());
  state("live");
  expect(mic.mode.value).toBe("locked");
  props.enabled = false;
  await nextTick();
  expect(mic.mode.value).toBe("idle");
  expect(native.stopMicrophone).toHaveBeenCalledWith(42);
});
it("system silencing stays visible without cancelling locked capture or repeating notices", () => {
  const notice = vi.fn();
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, notice);
  state("live");
  const warning = "系统暂停了此应用的收音";
  for (let i = 0; i < 10; i++) state("live", true, warning);
  expect(mic.warning.value).toBe(warning);
  expect(mic.level.value).toBe(0);
  expect(mic.mode.value).toBe("locked");
  expect(notice).toHaveBeenCalledTimes(1);
  expect(native.stopMicrophone).not.toHaveBeenCalled();
  state("live");
  expect(mic.warning.value).toBe("");
  expect(mic.level.value).toBe(.3);
  state("live", true, warning);
  expect(notice).toHaveBeenCalledTimes(2);
  state("idle", false);
  expect(mic.warning.value).toBe("");
});
it("restores a system-silenced session after reloading and still allows an immediate stop", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  state("live", true, "系统麦克风已静音");
  expect(mic.warning.value).toBe("系统麦克风已静音");
  mic.stop();
  expect(native.stopMicrophone).toHaveBeenCalledWith(42);
  expect(mic.warning.value).toBe("");
  expect(mic.mode.value).toBe("idle");
});
it("keeps a locked session through an unannotated reconnect snapshot", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  state("live", true);
  expect(mic.mode.value).toBe("locked");
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: {
    id: 42, phase: "idle", snapshot: true, level: 0, message: "",
  } }));
  expect(mic.mode.value).toBe("locked");
  expect(mic.lockedSession.value).toBe(true);
  mic.stop();
  expect(mic.mode.value).toBe("idle");
  expect(mic.lockedSession.value).toBe(false);
});

it("ignores a late locked event from an older request", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  state("live", true);
  mic.stop();
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: {
    id: 999, phase: "live", locked: true, snapshot: false, level: .8, message: "",
  } }));
  expect(mic.mode.value).toBe("idle");
  expect(mic.phase.value).toBe("idle");
  expect(mic.lockedSession.value).toBe(false);
});

it.each(["connecting", "reconnecting", "live"])("does not restore a stopped capture from a late %s snapshot", phase => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  state("live");
  mic.stop();
  state(phase);
  expect(mic.mode.value).toBe("idle");
  expect(mic.phase.value).toBe("idle");
  expect(mic.lockedSession.value).toBe(false);
  expect(mic.level.value).toBe(0);
});

it.each(["live", "idle", "error"])("keeps a new capture owned by its request when an old %s snapshot arrives", async phase => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  state("live"); mic.stop();
  mic.keyboard({ key: "Enter", repeat: false, preventDefault() {} } as KeyboardEvent, true);
  await vi.advanceTimersByTimeAsync(240);
  const current = native.startMicrophone.mock.lastCall![0];
  expect(current).not.toBe(42);
  state(phase, phase === "live");
  expect(mic.mode.value).toBe("holding");
  expect(mic.phase.value).toBe("connecting");
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: {
    id: current, phase: "live", locked: false, snapshot: false, level: .5,
  } }));
  expect(mic.recording.value).toBe(true);
  mic.stop();
  expect(native.stopMicrophone).toHaveBeenLastCalledWith(current);
});

it("does not adopt a snapshot with a non-boolean lock flag", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: {
    id: 42, phase: "live", locked: "yes", snapshot: true, level: .5,
  } }));
  expect(mic.mode.value).toBe("idle");
  expect(mic.lockedSession.value).toBe(false);
});

it("shows errors from malformed non-boolean snapshot metadata", () => {
  const notice = vi.fn();
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, notice);
  state("live");
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: {
    id: 42, phase: "error", locked: false, snapshot: "yes", level: 0, message: "收音失败",
  } }));
  expect(mic.phase.value).toBe("error");
  expect(notice).toHaveBeenCalledWith("收音失败");
});
it("keeps a terminal issue available after its notice and clears it only for a new capture", async () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  state("live");
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: 42, phase: "error", message: "设备不可用" } }));
  mic.stop(); expect(mic.lastIssue.value).toBe("设备不可用");
  mic.keyboard({ key: "Enter", repeat: false, preventDefault() {} } as KeyboardEvent, true);
  await vi.advanceTimersByTimeAsync(240);
  expect(mic.lastIssue.value).toBe("");
});
it("restores the service's last error without restarting capture or repeating the notice", () => {
  const notice = vi.fn();
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, notice);
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: 42, phase: "error", snapshot: true, message: "设备不可用" } }));
  expect(mic.lastIssue.value).toBe("设备不可用"); expect(notice).not.toHaveBeenCalled();
  expect(native.startMicrophone).not.toHaveBeenCalled(); expect(mic.lockedSession.value).toBe(false);
  mic.dismissIssue();
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: 42, phase: "error", snapshot: true, message: "设备不可用" } }));
  expect(mic.lastIssue.value).toBe("");
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: 42, phase: "error", snapshot: true, message: "新的问题" } }));
  expect(mic.lastIssue.value).toBe("新的问题");
});
it("refreshes Android recording access on return without checking it for every meter sample", () => {
  native.hasMicrophonePermission.mockReturnValue(false);
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  expect(mic.permissionGranted.value).toBe(false);
  native.hasMicrophonePermission.mockReturnValue(true);
  win.dispatchEvent(new Event("pageshow")); expect(mic.permissionGranted.value).toBe(true);
  state("live"); const checks = native.hasMicrophonePermission.mock.calls.length;
  for (let i = 0; i < 10; i++) win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: 42, phase: "live", level: .5 } }));
  expect(native.hasMicrophonePermission).toHaveBeenCalledTimes(checks);
});

it.each(["error", "permission", "idle"])("cleans a native %s terminal without clearing the service receipt with Stop", terminal => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn());
  state("live");
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: 42, phase: terminal, locked: false, message: "Native receipt" } }));
  expect(mic.mode.value).toBe("idle"); expect(mic.phase.value).toBe(terminal);
  expect(mic.level.value).toBe(0); expect(mic.lockedSession.value).toBe(false);
  expect(native.stopMicrophone).not.toHaveBeenCalled();
  if (terminal !== "idle") expect(mic.lastIssue.value).toBe("Native receipt");
});

it("explicitly restarts continuous capture from its help page with a new request and locks only after native readiness", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: false }, vi.fn());
  state("error", false, "Previous connection failed");
  expect(mic.restartLocked()).toBe(true);
  const current = native.startMicrophone.mock.lastCall![0];
  expect(current).not.toBe(42); expect(mic.mode.value).toBe("locked");
  expect(mic.phase.value).toBe("connecting"); expect(mic.lastIssue.value).toBe("");
  expect(native.lockMicrophone).not.toHaveBeenCalled();
  expect(mic.restartLocked()).toBe(false); expect(native.startMicrophone).toHaveBeenCalledTimes(1);
  state("error", false, "Old failure");
  expect(mic.phase.value).toBe("connecting"); expect(mic.lastIssue.value).toBe("");
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: current, phase: "connecting", locked: false } }));
  expect(native.lockMicrophone).toHaveBeenCalledExactlyOnceWith(current);
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: current, phase: "live", level: .5 } }));
  expect(native.lockMicrophone).toHaveBeenCalledTimes(1); expect(mic.recording.value).toBe(true);
  mic.stop(); expect(native.stopMicrophone).toHaveBeenCalledExactlyOnceWith(current);
});

it("requests permission from an explicit first start and waits for a second start after permission is granted", () => {
  native.hasMicrophonePermission.mockReturnValue(false);
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: false }, vi.fn());
  expect(mic.startLocked()).toBe(true);
  const first = native.startMicrophone.mock.lastCall![0];
  expect(mic.startLocked()).toBe(false);
  expect(native.lockMicrophone).not.toHaveBeenCalled();
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: first, phase: "permission", locked: false } }));
  native.hasMicrophonePermission.mockReturnValue(true);
  win.dispatchEvent(new Event("pageshow"));
  expect(mic.mode.value).toBe("idle");
  expect(native.startMicrophone).toHaveBeenCalledTimes(1);
  expect(native.lockMicrophone).not.toHaveBeenCalled();
  expect(mic.startLocked()).toBe(true);
  const current = native.startMicrophone.mock.lastCall![0];
  expect(current).not.toBe(first);
  for (const phase of ["connecting", "live"])
    win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: current, phase, locked: false } }));
  expect(native.lockMicrophone).toHaveBeenCalledExactlyOnceWith(current);
  expect(mic.startLocked()).toBe(false);
  mic.stop();
  expect(native.stopMicrophone).toHaveBeenCalledExactlyOnceWith(current);
});

it("cancels a queued first-start lock on Stop and rejects late snapshots", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: false }, vi.fn());
  expect(mic.startLocked()).toBe(true);
  const current = native.startMicrophone.mock.lastCall![0];
  mic.stop();
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: current, phase: "connecting", locked: true, snapshot: true } }));
  expect(mic.mode.value).toBe("idle");
  expect(native.lockMicrophone).not.toHaveBeenCalled();
});

it.each(["background", "disabled", "denied", "retained-error"])("rejects a first start in an ineligible %s state", reason => {
  const mic = useMicrophone({ allowed: reason !== "denied", enabled: reason !== "disabled", activePage: false }, vi.fn());
  if (reason === "background") Object.assign(document, { hidden: true });
  if (reason === "retained-error") state("error", false, "Review the previous failure");
  expect(mic.startLocked()).toBe(false);
  expect(native.startMicrophone).not.toHaveBeenCalled();
});

it("stopping a manual restart cancels its queued lock and ignores old snapshots", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: false }, vi.fn());
  state("error", false, "Previous connection failed"); mic.restartLocked();
  const current = native.startMicrophone.mock.lastCall![0]; mic.stop();
  state("live");
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: current, phase: "connecting", snapshot: true, locked: true } }));
  expect(mic.mode.value).toBe("idle"); expect(mic.phase.value).toBe("idle");
  expect(native.lockMicrophone).not.toHaveBeenCalled();
  expect(native.stopMicrophone).toHaveBeenCalledExactlyOnceWith(current);
});

it("a permission response ends a manual restart without implicitly starting after permission is granted", () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: false }, vi.fn());
  state("error", false, "Previous connection failed"); mic.restartLocked();
  const current = native.startMicrophone.mock.lastCall![0];
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id: current, phase: "permission", locked: false, message: "" } }));
  win.dispatchEvent(new Event("pageshow"));
  expect(mic.mode.value).toBe("idle"); expect(mic.phase.value).toBe("permission"); expect(native.startMicrophone).toHaveBeenCalledTimes(1);
  expect(native.lockMicrophone).not.toHaveBeenCalled(); expect(native.stopMicrophone).not.toHaveBeenCalled();
});

it.each(["permission", "background", "disabled", "no-issue"])("does not manually restart in an ineligible %s state", reason => {
  const mic = useMicrophone({ allowed: true, enabled: reason !== "disabled", activePage: false }, vi.fn());
  if (reason !== "no-issue") state("error", false, "Previous connection failed");
  if (reason === "permission") native.hasMicrophonePermission.mockReturnValue(false);
  if (reason === "background") Object.assign(document, { hidden: true });
  expect(mic.restartLocked()).toBe(false);
  expect(native.startMicrophone).not.toHaveBeenCalled(); expect(native.lockMicrophone).not.toHaveBeenCalled();
});

it.each(["live", "error"])("preserves bootstrap restoration of a %s Service snapshot through initial connection prop changes", async phase => {
  const props = reactive({ allowed: false, enabled: false, activePage: true, connecting: false });
  const mic = useMicrophone({
    get allowed() { return props.connecting && props.allowed; },
    get enabled() { return props.enabled; },
    get activePage() { return props.activePage; },
  }, vi.fn());
  props.connecting = true; await nextTick();
  props.allowed = true; props.enabled = true; await nextTick();
  state(phase, phase === "live", phase === "error" ? "Retained Service failure" : "");
  expect(mic.phase.value).toBe(phase);
  expect(mic.mode.value).toBe(phase === "live" ? "locked" : "idle");
  if (phase === "error") expect(mic.lastIssue.value).toBe("Retained Service failure");
  expect(native.stopMicrophone).not.toHaveBeenCalled();
});
