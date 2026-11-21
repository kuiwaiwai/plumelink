import { invoke, isTauri } from "@tauri-apps/api/core";
import { createApp, watch } from "vue";
import { initializeLanguage, locale } from "../../shared/i18n";
import App from "./App.vue";

window.addEventListener("contextmenu", (event) => event.preventDefault());

initializeLanguage();
if (isTauri()) {
  watch(locale, value => {
    void invoke("set_ui_language", { locale: value }).catch(console.error);
  }, { immediate: true });
}
createApp(App).mount("#app");
