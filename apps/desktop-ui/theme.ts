import { ref, watch } from "vue";

export type DesktopTheme = "system" | "light" | "dark";
const key = "plumelink-desktop-theme";
let saved: string | null = null;
try { saved = localStorage.getItem(key); } catch { /* Storage can be unavailable. */ }
export const desktopTheme = ref<DesktopTheme>(saved === "light" || saved === "dark" ? saved : "system");

watch(desktopTheme, value => {
  if (value === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = value;
  try { localStorage.setItem(key, value); } catch { /* Keep the session choice. */ }
}, { immediate: true });
