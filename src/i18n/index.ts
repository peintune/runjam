import { reactive } from "vue";
import en, { type TranslationKey } from "./locales/en";
import zhCN from "./locales/zh-CN";

export type Locale = "en-US" | "zh-CN";

const messages: Record<Locale, Record<TranslationKey, string>> = {
  "en-US": en,
  "zh-CN": zhCN,
};

const STORAGE_KEY = "runjam.locale";

function detectInitialLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "en-US" || saved === "zh-CN") return saved;
  } catch {
    // localStorage unavailable — fall through
  }
  // Default to the system language; fall back to English.
  try {
    const navLang = (navigator.language || "en-US").toLowerCase();
    if (navLang.startsWith("zh")) return "zh-CN";
  } catch {
    // ignore
  }
  return "en-US";
}

/** Reactive locale singleton — reading `locale` inside templates/computed
 *  tracks the dependency, so switching language re-renders automatically. */
const state = reactive<{ locale: Locale }>({ locale: detectInitialLocale() });

/**
 * Follow language changes made in ANOTHER window of the app.
 *
 * The pet windows are separate webviews under the same origin: they detect the
 * locale on boot, but a language switch in the main window would otherwise
 * leave them showing the old one. `storage` only fires for other windows, so
 * this never reacts to our own `setLocale`.
 */
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key !== STORAGE_KEY) return;
    if (e.newValue !== "en-US" && e.newValue !== "zh-CN") return;
    if (e.newValue === state.locale) return;
    state.locale = e.newValue;
  });
}

/** Translate a key. Params interpolate `{name}` placeholders. */
export function t(key: TranslationKey, params?: Record<string, string | number>): string {
  // Touch `state.locale` so callers that run inside Vue render effects
  // (templates, computed) re-evaluate when the language changes.
  const locale = state.locale;
  let str: string = messages[locale][key] ?? messages["en-US"][key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      str = str.split(`{${k}}`).join(String(v));
    }
  }
  return str;
}

/** The current locale (reactive). */
export function currentLocale(): Locale {
  return state.locale;
}

/** Switch the app language and persist the choice. */
export function setLocale(locale: Locale): void {
  state.locale = locale;
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    // non-fatal
  }
}

export type { TranslationKey };
export default t;
