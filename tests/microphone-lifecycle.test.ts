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
const native = { startMicrophone: vi.fn(), stopMicrophone: vi.fn(), lockMicrophone: vi.fn(), syncSession: vi.fn() };
beforeEach(() => {
  vi.useFakeTimers(); vi.clearAllMocks(); lifecycle.destroy = [];
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
it("meter snapshots do not reset the two-second stop gesture", async () => {
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, vi.fn()); state("live");
  mic.keyboard({ key: " ", repeat: false, preventDefault() {} } as KeyboardEvent, true);
  for (let i = 0; i < 21; i++) { await vi.advanceTimersByTimeAsync(100); state("live"); }
  expect(native.stopMicrophone).toHaveBeenCalledWith(42);
});
it("a notification stop is reflected after returning, and revoked permission stops capture", async () => {
  const props = reactive({ allowed: true, enabled: true, activePage: true });
  const mic = useMicrophone(props, vi.fn()); state("live"); state("idle", false);
  expect(mic.mode.value).toBe("idle");
  state("live"); props.allowed = false; await nextTick();
  expect(mic.mode.value).toBe("idle"); expect(native.stopMicrophone).toHaveBeenCalledWith(42);
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

it("shows errors from malformed non-boolean snapshot metadata", () => {
  const notice = vi.fn();
  const mic = useMicrophone({ allowed: true, enabled: true, activePage: true }, notice);
  state("live");
  win.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: {
    id: 42, phase: "error", locked: false, snapshot: "yes", level: 0, message: "收音失败",
  } }));
  expect(mic.phase.value).toBe("idle");
  expect(notice).toHaveBeenCalledWith("收音失败");
});
