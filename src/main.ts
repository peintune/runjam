import { createApp } from "vue";
import { createPinia } from "pinia";
import router from "./router";
import App from "./App.vue";
import "./assets/styles/main.css";
import { initDiag } from "./lib/diag";
import { t } from "./i18n";
import { useThemeStore } from "./stores/useThemeStore";

initDiag();

const app = createApp(App);
app.config.globalProperties.$t = t;
app.use(createPinia());
// Initialize the theme store before mounting so the `dark` class is applied
// to <html> right away (also guards against a flash of the wrong theme).
useThemeStore();
app.use(router);
app.mount("#app");
