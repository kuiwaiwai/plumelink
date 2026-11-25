import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { computed, effectScope, nextTick, ref, type EffectScope } from "vue";
import { useMobileSettings } from "../apps/mobile/composables/useMobileSettings";
import type { AndroidBridge } from "../apps/mobile/native/bridge";
import type { Destination } from "../apps/mobile/native/navigation";

vi.mock("vue", async original => ({ ...await original<typeof import("vue")>(), onMounted: () => {}, onUnmounted: () => {} }));
let scope: EffectScope;
let values: Map<string, string>;
const legacy = [{ action: "presentation", icon: "heart" }, { action: "fullscreen", icon: "bell" }, { action: "mute", icon: "star" }];
beforeEach(() => {
  values = new Map();
  legacy.forEach((item, index) => { const slot = ["a", "b", "c"][index]; values.set("plumelink-shortcut-" + slot, item.action); values.set("plumelink-shortcut-icon-" + slot, item.icon); });
  vi.stubGlobal("localStorage", { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value) });
  vi.stubGlobal("window", { matchMedia: () => ({ matches: false }) });
  vi.stubGlobal("document", { documentElement: { dataset: {} }, querySelector: () => null, hidden: false });
  scope = effectScope();
});
afterEach(() => { scope.stop(); vi.unstubAllGlobals(); });
function settings(saved: string) {
  const save = vi.fn();
  const android = { getMobilePreferences: () => saved.startsWith("[") ? JSON.stringify({ shortcuts: JSON.parse(saved) }) : saved, setMobilePreferences: save } as unknown as AndroidBridge;
  const model = scope.run(() => useMobileSettings(android, computed(() => false), ref<Destination>("remote"), vi.fn()))!;
  return { model, save };
}
it("migrates all existing actions and customized icons when native preferences are empty", () => {
  const { model, save } = settings("");
  expect([model.shortcutA.value, model.shortcutB.value, model.shortcutC.value]).toEqual(legacy.map(item => item.action));
  expect(JSON.parse(save.mock.calls[0][0]).shortcuts).toEqual(legacy);
});
it("restores native preferences over stale WebView storage and saves a preset as one snapshot", async () => {
  const saved = [{ action: "key:ctrl+shift+f5", icon: "keyboard" }, { action: "slideblack", icon: "heart" }, { action: "slidenext", icon: "right" }];
  const { model, save } = settings(JSON.stringify(saved));
  expect(model.shortcutA.value).toBe(saved[0].action);
  expect(model.shortcutIconB.value).toBe("heart");
  expect(values.get("plumelink-shortcut-a")).toBe(saved[0].action);
  save.mockClear();
  model.shortcutA.value = "slideprevious"; model.shortcutB.value = "slideblack"; model.shortcutC.value = "slidenext";
  model.shortcutIconA.value = "left"; model.shortcutIconB.value = "monitor"; model.shortcutIconC.value = "right";
  await nextTick();
  expect(save).toHaveBeenCalledOnce();
  expect(JSON.parse(save.mock.calls[0][0]).shortcuts).toEqual([{ action: "slideprevious", icon: "left" }, { action: "slideblack", icon: "monitor" }, { action: "slidenext", icon: "right" }]);
});
it("ignores malformed or unsupported native preferences without discarding valid legacy choices", () => {
  const { model, save } = settings(JSON.stringify([{ action: "unknown-command", icon: "heart" }, ...legacy.slice(1)]));
  expect(model.shortcutA.value).toBe("presentation");
  expect(JSON.parse(save.mock.calls[0][0]).shortcuts).toEqual(legacy);
});

const controls = { sensitivity: 2.4, keepAwake: false, haptics: false, hardwareVolume: true, longPressDrag: false,
  scrollDirection: "follow", responseMode: "responsive", padOnRight: true, focusMode: true, controlMode: "dpad", dialSize: "large", microphoneMode: "voice" };
it("migrates legacy control preferences without absorbing language, appearance, or background authorization", () => {
  for (const [key, value] of Object.entries({ sensitivity: "2.4", "keep-awake": "false", haptics: "false", "hardware-volume": "true",
    "long-press-drag": "false", "scroll-direction": "follow", "response-mode": "responsive", "pad-side": "right",
    "focus-mode": "true", "control-mode": "dpad", "dial-size": "large", "microphone-mode": "voice", theme: "dark", "keep-alive": "false" })) values.set("plumelink-" + key, value);
  const { model, save } = settings("");
  expect(JSON.parse(save.mock.calls[0][0])).toEqual({ version: 3, ...controls, shortcuts: legacy, shortcutPlans: [] });
  expect(model.theme.value).toBe("dark"); expect(model.keepAlive.value).toBe(false);
});
it("restores the complete native control snapshot over stale WebView mirrors", () => {
  const { model } = settings(JSON.stringify({ ...controls, shortcuts: legacy }));
  for (const [key, value] of Object.entries(controls)) expect(model[key as keyof typeof controls].value, key).toBe(value);
  expect(values.get("plumelink-pad-side")).toBe("right");
  expect(values.get("plumelink-scroll-direction")).toBe("follow");
  expect(values.get("plumelink-haptics")).toBe("false");
});
it("batches rapid control changes with the existing shortcuts after value normalization", async () => {
  const { model, save } = settings(""); save.mockClear();
  model.controlMode.value = "dpad"; model.dialSize.value = "large"; model.scrollDirection.value = "follow";
  model.haptics.value = false; model.sensitivity.value = 3.7;
  await nextTick();
  expect(save).toHaveBeenCalledOnce();
  expect(JSON.parse(save.mock.calls[0][0])).toMatchObject({ controlMode: "dpad", dialSize: "large", scrollDirection: "follow", haptics: false, sensitivity: 3, shortcuts: legacy });
});
it("does not turn invalid native scalar types into enabled settings or overwrite global preferences", () => {
  const { model } = settings(JSON.stringify({ controlMode: "dpad", dialSize: "enormous", haptics: "false", sensitivity: 99, theme: "dark", keepAlive: false }));
  expect(model.controlMode.value).toBe("dpad"); expect(model.dialSize.value).toBe("standard");
  expect(model.haptics.value).toBe(true); expect(model.sensitivity.value).toBe(1.5);
  expect(model.theme.value).toBe("system"); expect(model.keepAlive.value).toBe(true);
});
it("keeps native settings available when WebView storage cannot be read or written", () => {
  vi.stubGlobal("localStorage", { getItem: () => { throw Error("unavailable"); }, setItem: () => { throw Error("unavailable"); } });
  const { model, save } = settings(JSON.stringify({ ...controls, shortcuts: legacy }));
  expect(model.controlMode.value).toBe("dpad"); expect(model.shortcutIconA.value).toBe("heart");
  expect(JSON.parse(save.mock.calls[0][0])).toEqual({ version: 3, ...controls, shortcuts: legacy, shortcutPlans: [] });
});
it("restores computer-specific plans from native storage without importing another computer's WebView mirror", async () => {
  values.set("plumelink-shortcut-plans", JSON.stringify([{ id: "p" + "f".repeat(32), name: "Other PC", shortcuts: legacy }]));
  const saved = [{ id: "p" + "a".repeat(32), name: "Work", shortcuts: legacy }];
  const { model, save } = settings(JSON.stringify({ ...controls, shortcuts: legacy, shortcutPlans: saved }));
  expect(model.shortcutPlans.value).toEqual(saved);
  save.mockClear(); model.shortcutPlans.value = [];
  await nextTick(); expect(JSON.parse(save.mock.calls[0][0]).shortcutPlans).toEqual([]);
});
it("applies all three customized icons atomically even if an icon equals the previous action's default", async () => {
  const { model, save } = settings(JSON.stringify({ ...controls, shortcuts: [{ action: "copy", icon: "layers" }, { action: "paste", icon: "keyboard" }, { action: "undo", icon: "left" }] }));
  save.mockClear();
  const values = [{ action: "paste", icon: "layers" }, { action: "undo", icon: "keyboard" }, { action: "copy", icon: "left" }];
  model.applyShortcuts(values); await nextTick();
  expect(save).toHaveBeenCalledOnce(); expect(JSON.parse(save.mock.calls[0][0]).shortcuts).toEqual(values);
});
