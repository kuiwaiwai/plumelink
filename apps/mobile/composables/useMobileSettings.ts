import { computed, onMounted, onUnmounted, ref, watch, type ComputedRef, type Ref } from "vue";
import { storage } from "../storage";
import type { Destination } from "../native/navigation";
import type { AndroidBridge } from "../native/bridge";
import { isShortcutValue } from "../shortcuts";
import { defaultShortcutIcon, shortcutIconNames } from "../shortcut-icons";
import { controlPreferenceKeys, controlPreferenceStorage, controlStorageValue, validControlPreference } from "../control-preferences";
import { parseShortcutPlans, validShortcutSlots, type ShortcutPlan, type ShortcutSlot } from "../shortcut-plans";



export function useMobileSettings(
  android: AndroidBridge | undefined,
  online: ComputedRef<boolean>,
  destination: Ref<Destination>,
  setResponseMode: (mode: "balanced" | "responsive") => void,
) {
  const savedSensitivity = Number(storage.getItem("plumelink-sensitivity") || "1.5");
  const sensitivity = ref(Number.isFinite(savedSensitivity) ? Math.min(3, Math.max(0.5, savedSensitivity)) : 1.5);
  const keepAwake = ref(storage.getItem("plumelink-keep-awake") !== "false");
  const nativeKeepAlive = android?.getKeepAlivePreference?.();
  const previousKeepAlive = storage.getItem("plumelink-keep-alive");
  const keepAlive = ref((nativeKeepAlive || previousKeepAlive) !== "false");
  if (nativeKeepAlive === "" && (previousKeepAlive === "true" || previousKeepAlive === "false")) {
    android?.setKeepAlivePreference?.(keepAlive.value);
  }
  const haptics = ref(storage.getItem("plumelink-haptics") !== "false");
  const hardwareVolume = ref(storage.getItem("plumelink-hardware-volume") === "true");
  const savedTheme = android?.getThemePreference?.() || storage.getItem("plumelink-theme");
  const theme = ref(savedTheme === "light" || savedTheme === "dark" ? savedTheme : "system");
  const savedShortcutA = storage.getItem("plumelink-shortcut-a");
  const savedShortcutB = storage.getItem("plumelink-shortcut-b");
  const savedShortcutC = storage.getItem("plumelink-shortcut-c");
  const shortcutA = ref(isShortcutValue(savedShortcutA) ? savedShortcutA : "taskview");
  const shortcutB = ref(isShortcutValue(savedShortcutB) ? savedShortcutB : "notifications");
  const shortcutC = ref(isShortcutValue(savedShortcutC) ? savedShortcutC : "nextapp");
  const iconFor = (slot: string, action: string) => {
    const saved = storage.getItem(`plumelink-shortcut-icon-${slot}`);
    return saved && shortcutIconNames.includes(saved) ? saved : defaultShortcutIcon(action);
  };
  const shortcutIconA = ref(iconFor("a", shortcutA.value));
  const shortcutIconB = ref(iconFor("b", shortcutB.value));
  const shortcutIconC = ref(iconFor("c", shortcutC.value));
  const shortcutActions = [shortcutA, shortcutB, shortcutC];
  const shortcutIcons = [shortcutIconA, shortcutIconB, shortcutIconC];
  const shortcutPlans = ref<ShortcutPlan[]>([]);
  const longPressDrag = ref(storage.getItem("plumelink-long-press-drag") !== "false");
  const scrollDirection = ref(storage.getItem("plumelink-scroll-direction") === "follow" ? "follow" : "opposite");
  const savedResponseMode = storage.getItem("plumelink-response-mode");
  const responseMode = ref(savedResponseMode === "responsive" ? "responsive" : "balanced");
  const padOnRight = ref(storage.getItem("plumelink-pad-side") === "right");
  const focusMode = ref(storage.getItem("plumelink-focus-mode") === "true");
  const controlMode = ref(storage.getItem("plumelink-control-mode") === "dpad" ? "dpad" : "trackpad");
  const savedDialSize = storage.getItem("plumelink-dial-size");
  const dialSize = ref(savedDialSize === "small" || savedDialSize === "large" ? savedDialSize : "standard");
  const microphoneMode = ref(storage.getItem("plumelink-microphone-mode") === "voice" ? "voice" : "standard");
  const controlModels = { sensitivity, keepAwake, haptics, hardwareVolume, longPressDrag, scrollDirection, responseMode, padOnRight, focusMode, controlMode, dialSize, microphoneMode };
  try {
    const saved = android?.getMobilePreferences?.();
    if (saved && saved.length <= 16384) {
      const input: unknown = JSON.parse(saved);
      if (input && typeof input === "object" && !Array.isArray(input)) {
        const preferences = input as Record<string, unknown>;
        shortcutPlans.value = parseShortcutPlans(preferences.shortcutPlans) || [];
        for (const key of controlPreferenceKeys) if (validControlPreference(key, preferences[key])) {
          (controlModels[key] as Ref<unknown>).value = preferences[key];
          storage.setItem("plumelink-" + controlPreferenceStorage[key], controlStorageValue(key, preferences[key]));
        }
        const values = preferences.shortcuts;
        if (Array.isArray(values) && values.length === 3 && values.every(item => item && typeof item === "object"
          && isShortcutValue(item.action) && typeof item.icon === "string" && shortcutIconNames.includes(item.icon))) {
          values.forEach((item, index) => {
            shortcutActions[index].value = item.action; shortcutIcons[index].value = item.icon;
            const slot = ["a", "b", "c"][index];
            storage.setItem("plumelink-shortcut-" + slot, item.action);
            storage.setItem("plumelink-shortcut-icon-" + slot, item.icon);
          });
        }
      }
    }
  } catch {}
  const immersive = computed(() => focusMode.value && destination.value === "remote");

  const systemDark = window.matchMedia("(prefers-color-scheme: dark)");
  const systemDarkEnabled = ref(android?.isSystemDark?.() ?? systemDark.matches);
  const darkAppearance = computed(() => theme.value === "dark" || (theme.value === "system" && systemDarkEnabled.value));

  function syncAppearance() {
    systemDarkEnabled.value = android?.isSystemDark?.() ?? systemDark.matches;
    const dark = darkAppearance.value;
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", dark ? "#111318" : "#f5f6fa");
    android?.setAppearance?.(dark);
  }

  async function syncWakeLock() {
    android?.setKeepAwake?.(online.value && keepAwake.value && !document.hidden);
  }

  watch(padOnRight, right => storage.setItem("plumelink-pad-side", right ? "right" : "left"));
  watch(hardwareVolume, value => storage.setItem("plumelink-hardware-volume", String(value)));
  watch(controlMode, value => storage.setItem("plumelink-control-mode", value));
  watch(dialSize, value => storage.setItem("plumelink-dial-size", value));
  watch(focusMode, value => storage.setItem("plumelink-focus-mode", String(value)));
  watch(immersive, value => android?.setImmersive?.(value));
  watch(sensitivity, value => {
    const normalized = Number.isFinite(value) ? Math.min(3, Math.max(0.5, value)) : 1.5;
    if (normalized !== value) { sensitivity.value = normalized; return; }
    storage.setItem("plumelink-sensitivity", String(normalized));
  });
  watch(keepAwake, value => {
    storage.setItem("plumelink-keep-awake", String(value));
    void syncWakeLock();
  });
  watch(keepAlive, value => {
    if (android?.setKeepAlivePreference) android.setKeepAlivePreference(value);
    else storage.setItem("plumelink-keep-alive", String(value));
  });
  watch(haptics, value => storage.setItem("plumelink-haptics", String(value)));
  watch(theme, value => {
    storage.setItem("plumelink-theme", value);
    android?.setThemePreference?.(value);
    if (value === "system") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = value;
    syncAppearance();
  }, { immediate: true });
  [
    { slot: "a", action: shortcutA, icon: shortcutIconA, fallback: "taskview" },
    { slot: "b", action: shortcutB, icon: shortcutIconB, fallback: "notifications" },
    { slot: "c", action: shortcutC, icon: shortcutIconC, fallback: "nextapp" },
  ].forEach(({ slot, action, icon, fallback }) => {
    watch(action, (value, previous) => {
      storage.setItem("plumelink-shortcut-" + slot, isShortcutValue(value) ? value : fallback);
      if (icon.value === defaultShortcutIcon(previous)) icon.value = defaultShortcutIcon(value);
    }, { flush: "sync" });
  });
  watch(shortcutIconA, value => storage.setItem("plumelink-shortcut-icon-a", shortcutIconNames.includes(value) ? value : "star"));
  watch(shortcutIconB, value => storage.setItem("plumelink-shortcut-icon-b", shortcutIconNames.includes(value) ? value : "star"));
  watch(shortcutIconC, value => storage.setItem("plumelink-shortcut-icon-c", shortcutIconNames.includes(value) ? value : "star"));
  watch([...Object.values(controlModels), ...shortcutActions, ...shortcutIcons, shortcutPlans], () => {
    try { android?.setMobilePreferences?.(JSON.stringify({
      version: 3,
      ...Object.fromEntries(controlPreferenceKeys.map(key => [key, controlModels[key].value])),
      shortcuts: shortcutActions.map((action, index) => ({ action: action.value, icon: shortcutIcons[index].value })),
      shortcutPlans: shortcutPlans.value,
    })); }
    catch {}
  }, { immediate: true, flush: "post" });
  watch(longPressDrag, value => storage.setItem("plumelink-long-press-drag", String(value)));
  watch(microphoneMode, value => storage.setItem("plumelink-microphone-mode", value));
  watch(scrollDirection, value => storage.setItem("plumelink-scroll-direction", value));
  watch(responseMode, value => {
    storage.setItem("plumelink-response-mode", value);
    setResponseMode(value === "responsive" ? "responsive" : "balanced");
  }, { immediate: true });

  onMounted(() => {
    android?.setImmersive?.(immersive.value);
    window.addEventListener("plumelink-appearance", syncAppearance);
    systemDark.addEventListener("change", syncAppearance);
  });

  onUnmounted(() => {
    android?.setKeepAwake?.(false);
    window.removeEventListener("plumelink-appearance", syncAppearance);
    systemDark.removeEventListener("change", syncAppearance);
  });

  return {
    shortcutPlans,
    applyShortcuts(values: ShortcutSlot[]) {
      if (!validShortcutSlots(values)) return;
      values.forEach((slot, index) => { shortcutActions[index].value = slot.action; });
      values.forEach((slot, index) => { shortcutIcons[index].value = slot.icon; });
    },
    sensitivity,
    keepAwake,
    keepAlive,
    haptics,
    hardwareVolume,
    theme,
    darkAppearance,
    shortcutA,
    shortcutB,
    shortcutC,
    shortcutIconA,
    shortcutIconB,
    shortcutIconC,
    longPressDrag,
    scrollDirection,
    responseMode,
    padOnRight,
    focusMode,
    controlMode,
    dialSize,
    microphoneMode,
    immersive,
    syncWakeLock,
  };
}
