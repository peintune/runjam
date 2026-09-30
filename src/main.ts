import { createApp } from "vue";
import { createPinia } from "pinia";
import router from "./router";
import App from "./App.vue";
import PetIcon from "./components/pet/PetIcon.vue";
import PetChat from "./components/pet/PetChat.vue";
import "./assets/styles/main.css";
import { initDiag } from "./lib/diag";
import { t } from "./i18n";
import { useThemeStore } from "./stores/useThemeStore";

initDiag();

// ── Window routing ────────────────────────────────────────────────────────
// The desktop-pet windows (`pet-icon`, `pet-chat`) load this same bundle but
// must NOT mount the full app shell: they are tiny, always-on-top surfaces with
// no routing, no session list and no onboarding. The backend opens them with a
// `?window=pet-*` query parameter (see commands/pet_cmd.rs), so we branch here
// before touching the router or the main App component.
//
// Both pet windows still need Pinia (the toast store etc.) and the theme store
// (so they follow light/dark), but they skip the router entirely.
const windowRole = new URLSearchParams(window.location.search).get("window");
const petComponent =
  windowRole === "pet-icon" ? PetIcon : windowRole === "pet-chat" ? PetChat : null;

const app = createApp(petComponent ?? App);
app.config.globalProperties.$t = t;
app.use(createPinia());
// Initialize the theme store before mounting so the `dark` class is applied
// to <html> right away (also guards against a flash of the wrong theme).
useThemeStore();
if (!petComponent) {
  app.use(router);
}
app.mount("#app");
