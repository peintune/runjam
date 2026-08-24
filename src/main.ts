import { createApp } from "vue";
import { createPinia } from "pinia";
import router from "./router";
import App from "./App.vue";
import "./assets/styles/main.css";
import { initDiag } from "./lib/diag";
import { t } from "./i18n";

initDiag();

const app = createApp(App);
app.config.globalProperties.$t = t;
app.use(createPinia());
app.use(router);
app.mount("#app");
