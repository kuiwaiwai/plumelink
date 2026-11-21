import { computed, onMounted, onUnmounted, ref, watch, type ComputedRef, type Ref } from "vue";
import { storage } from "../storage";
import type { Destination } from "../components/AppNavigation.vue";
import type { AndroidBridge } from "../native/bridge";
import { defaultShortcutIcon, shortcutIconNames } from "../shortcut-icons";

const SHORTCUT_VALUES = ["mute", "magnifier", "taskview", "notifications", "previousapp", "nextapp", "virtualleft", "virtualright"] as const;

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
  const savedTheme = android?.getThemePreference?.() || storage.getItem("plumelink-theme");
  const theme = ref(savedTheme === "light" || savedTheme === "dark" ? savedTheme : "system");
  const savedShortcutA = storage.getItem("plumelink-shortcut-a");
  const savedShortcutB = storage.getItem("plumelink-shortcut-b");
  const savedShortcutC = storage.getItem("plumelink-shortcut-c");
  const shortcutA = ref(SHORTCUT_VALUES.includes(savedShortcutA as typeof SHORTCUT_VALUES[number]) ? savedShortcutA! : "taskview");
  const shortcutB = ref(SHORTCUT_VALUES.includes(savedShortcutB as typeof SHORTCUT_VALUES[number]) ? savedShortcutB! : "notifications");
  const shortcutC = ref(SHORTCUT_VALUES.includes(savedShortcutC as typeof SHORTCUT_VALUES[number]) ? savedShortcutC! : "nextapp");
  const iconFor = (slot: string, action: string) => {
    const saved = storage.getItem(`plumelink-shortcut-icon-${slot}`);
    return saved && shortcutIconNames.includes(saved) ? saved : defaultShortcutIcon(action);
  };
  const shortcutIconA = ref(iconFor("a", shortcutA.value));
  const shortcutIconB = ref(iconFor("b", shortcutB.value));
  const shortcutIconC = ref(iconFor("c", shortcutC.value));
  const longPressDrag = ref(storage.getItem("plumelink-long-press-drag") !== "false");
  const scrollDirection = ref(storage.getItem("plumelink-scroll-direction") === "follow" ? "follow" : "opposite");
  const savedResponseMode = storage.getItem("plumelink-response-mode");
  const responseMode = ref(savedResponseMode === "responsive" ? "responsive" : "balanced");
  const padOnRight = ref(storage.getItem("plumelink-pad-side") === "right");
  const focusMode = ref(storage.getItem("plumelink-focus-mode") === "true");
  const controlMode = ref(storage.getItem("plumelink-control-mode") === "dpad" ? "dpad" : "trackpad");
  const savedDialSize = storage.getItem("plumelink-dial-size");
  const dialSize = ref(savedDialSize === "small" || savedDialSize === "large" ? savedDialSize : "standard");
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

  let wakeLock: { release(): Promise<void> } | null = null;
  let wakeGeneration = 0;

  async function syncWakeLock() {
    const ticket = ++wakeGeneration;
    if (android?.setKeepAwake) {
      android.setKeepAwake(online.value && keepAwake.value && !document.hidden);
      return;
    }
    if (wakeLock && (!online.value || !keepAwake.value || document.hidden)) {
      await wakeLock.release().catch(() => {});
      wakeLock = null;
    }
    if (!wakeLock && online.value && keepAwake.value && !document.hidden && "wakeLock" in navigator) {
      try {
        const acquired = await (navigator as Navigator & { wakeLock: { request(type: "screen"): Promise<{ release(): Promise<void> }> } }).wakeLock.request("screen");
        if (ticket !== wakeGeneration || !online.value || !keepAwake.value || document.hidden) await acquired.release();
        else wakeLock = acquired;
      } catch {
        wakeLock = null;
      }
    }
  }

  watch(padOnRight, right => storage.setItem("plumelink-pad-side", right ? "right" : "left"));
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
  watch(shortcutA, value => storage.setItem("plumelink-shortcut-a", SHORTCUT_VALUES.includes(value as typeof SHORTCUT_VALUES[number]) ? value : "taskview"));
  watch(shortcutB, value => storage.setItem("plumelink-shortcut-b", SHORTCUT_VALUES.includes(value as typeof SHORTCUT_VALUES[number]) ? value : "notifications"));
  watch(shortcutC, value => storage.setItem("plumelink-shortcut-c", SHORTCUT_VALUES.includes(value as typeof SHORTCUT_VALUES[number]) ? value : "nextapp"));
  watch(shortcutIconA, value => storage.setItem("plumelink-shortcut-icon-a", shortcutIconNames.includes(value) ? value : "star"));
  watch(shortcutIconB, value => storage.setItem("plumelink-shortcut-icon-b", shortcutIconNames.includes(value) ? value : "star"));
  watch(shortcutIconC, value => storage.setItem("plumelink-shortcut-icon-c", shortcutIconNames.includes(value) ? value : "star"));
  watch(longPressDrag, value => storage.setItem("plumelink-long-press-drag", String(value)));
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
    wakeGeneration++;
    window.removeEventListener("plumelink-appearance", syncAppearance);
    systemDark.removeEventListener("change", syncAppearance);
    void wakeLock?.release().catch(() => {});
  });

  return {
    sensitivity,
    keepAwake,
    keepAlive,
    haptics,
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
    immersive,
    syncWakeLock,
  };
}
