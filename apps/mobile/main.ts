import { createApp } from "vue";
import App from "./App.vue";

import { initializeLanguage } from "../../shared/i18n";
import { androidBridge } from "./native/bridge";

const android = androidBridge();
if (android?.renderNativeUi) {
  initializeLanguage(android);
  createApp(App).mount("#app");
}
