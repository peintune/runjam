import { ref, watch } from "vue";
import { defineStore } from "pinia";
import { clearMarkdownCaches } from "../composables/useMarkdown";

export type ThemeMode = "light" | "dark";

const STORAGE_KEY = "runjam.theme";

function getInitialTheme(): ThemeMode {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "dark" || saved === "light") return saved;
  } catch {
    // ignore storage errors
  }
  return "dark";
}

function applyTheme(mode: ThemeMode): void {
  document.documentElement.classList.toggle("dark", mode === "dark");
  try {
    localStorage.setItem(STORAGE_KEY, mode);
  } catch {
    // ignore storage errors
  }
}

/**
 * Global theme store (light / dark).
 * Applies the `dark` class on <html> and persists the choice to localStorage.
 */
export const useThemeStore = defineStore("theme", () => {
  const theme = ref<ThemeMode>(getInitialTheme());

  // Apply immediately on store creation so the app boots with the right theme.
  applyTheme(theme.value);

  watch(theme, (val) => {
    applyTheme(val);
    // Markdown render cache is keyed by theme; drop stale entries on switch.
    clearMarkdownCaches();
  });

  function setTheme(mode: ThemeMode) {
    theme.value = mode;
  }

  function toggle() {
    theme.value = theme.value === "dark" ? "light" : "dark";
  }

  return { theme, setTheme, toggle };
});
