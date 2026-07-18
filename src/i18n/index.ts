import { ref, watch } from "vue";
import en from "./en";
import zh from "./zh";
import type { Locale } from "./en";

export type Lang = "en" | "zh";

const STORAGE_KEY = "runjam-lang";

function getBrowserLang(): Lang {
  if (typeof window === "undefined") return "en";
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === "zh" || stored === "en") return stored;
  // Detect browser language
  const nav = navigator.language.toLowerCase();
  if (nav.startsWith("zh")) return "zh";
  return "en";
}

const locales: Record<Lang, Locale> = { en, zh };

// Singleton reactive state
const currentLang = ref<Lang>(getBrowserLang());

watch(currentLang, (val) => {
  localStorage.setItem(STORAGE_KEY, val);
  // Update html lang attribute
  document.documentElement.lang = val === "zh" ? "zh-CN" : "en";
});

// Initialize html lang attribute
if (typeof document !== "undefined") {
  document.documentElement.lang = currentLang.value === "zh" ? "zh-CN" : "en";
}

/**
 * Lightweight i18n composable.
 * Usage: const { t, lang, toggleLang } = useI18n()
 *        <span>{{ t('nav.features') }}</span>
 */
export function useI18n() {
  function t(key: string): string {
    const parts = key.split(".");
    let value: any = locales[currentLang.value];
    for (const part of parts) {
      if (value == null) return key;
      value = value[part];
    }
    return typeof value === "string" ? value : key;
  }

  function toggleLang() {
    currentLang.value = currentLang.value === "en" ? "zh" : "en";
  }

  return {
    t,
    lang: currentLang,
    toggleLang,
  };
}

export { currentLang, locales };
export type { Locale };
