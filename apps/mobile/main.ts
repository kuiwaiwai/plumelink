import { createApp } from "vue";
import App from "./App.vue";

import { initializeLanguage } from "../../shared/i18n";
import { androidBridge } from "./native/bridge";

initializeLanguage(androidBridge());
const application = createApp(App);
application.mount("#app");

document.addEventListener("contextmenu", (event) => event.preventDefault());
