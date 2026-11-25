import servicePatterns from "./locales/service-patterns.json";
import { computed, ref, watch } from "vue";
import english from "./locales/en.json";
import traditional from "./locales/zh-Hant.json";

type Locale = "zh-CN" | "zh-TW" | "en";
type Preference = Locale | "system";
const storageKey = "plumelink-language";
const preference = ref<Preference>("system");
const systemLanguage = ref<Locale>("zh-CN");

export function resolveLocale(languages: readonly string[]): Locale {
  for (const tag of languages) {
    try {
      const locale = new Intl.Locale(tag);
      if (locale.language === "en") return "en";
      if (locale.language === "zh") return locale.maximize().script === "Hant" ? "zh-TW" : "zh-CN";
    } catch { /* Skip malformed language tags. */ }
  }
  return "en";
}

export const locale = computed(() => preference.value === "system" ? systemLanguage.value : preference.value);
function valid(value: unknown): value is Preference {
  return value === "system" || value === "en" || value === "zh-CN" || value === "zh-TW";
}

type NativeLanguage = {
  getLanguagePreference?(): string;
  setLanguagePreference?(value: string): void;
  getSystemLanguage?(): string;
};
let native: NativeLanguage | undefined;
export function initializeLanguage(bridge?: NativeLanguage) {
  native = bridge;
  let saved: unknown;
  try { saved = native?.getLanguagePreference?.() ?? localStorage.getItem(storageKey); } catch { /* Storage is optional. */ }
  if (valid(saved)) preference.value = saved;
  const detect = () => {
    const language = native?.getSystemLanguage?.();
    systemLanguage.value = resolveLocale(language ? language.split(",") : navigator.languages);
  };
  detect();
  window.addEventListener("languagechange", detect);
  watch(locale, value => {
    document.documentElement.lang = value;
    document.title = t("羽连 PlumeLink");
  }, { immediate: true });
}
export const languagePreference = computed({
  get: () => preference.value,
  set(value: string) {
    if (!valid(value)) return;
    preference.value = value;
    native?.setLanguagePreference?.(value);
    try { localStorage.setItem(storageKey, value); } catch { /* Keep the session preference. */ }
  },
});
export const languageOptions = computed(() => [
  { value: "system", label: t("跟随系统") },
  { value: "zh-CN", label: "简体中文" },
  { value: "zh-TW", label: "繁體中文" },
  { value: "en", label: "English" },
]);

export function t(key: keyof typeof english, ...args: unknown[]): string {
  const pattern = locale.value === "en" ? english[key] : locale.value === "zh-TW" ? traditional[key] : key;
  return pattern.replace(/\{(\d+)\}/g, (_, index: string) => String(args[Number(index)]));
}
type MessageKey = keyof typeof english;
const messageKeys = Object.keys(english) as MessageKey[];
const savedMessages = new Map<string, MessageKey>();
for (const key of messageKeys) {
  savedMessages.set(key, key);
  savedMessages.set(english[key], key);
  savedMessages.set(traditional[key], key);
}

// Translate known wrappers while preserving filenames, paths and OS details.
const serviceMessages = servicePatterns.map(pattern => {
  const [prefix, suffix] = pattern.split("{0}");
  return { key: pattern as MessageKey, prefix, suffix };
});
export function displayMessage(value?: string | null): string {
  if (!value) return "";
  const key = savedMessages.get(value);
  if (key) return t(key);
  const lines = value.split("\n");
  if (lines.length > 1 && lines.every(line => savedMessages.has(line)))
    return lines.map(line => t(savedMessages.get(line)!)).join("\n");
  for (const { key, prefix, suffix } of serviceMessages) {
    if (value.startsWith(prefix) && value.endsWith(suffix)) {
      return t(key, value.slice(prefix.length, value.length - suffix.length));
    }
  }
  return value;
}
