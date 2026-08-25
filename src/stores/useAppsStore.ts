import { ref, watch } from "vue";
import { defineStore } from "pinia";

export interface AppItem {
  id: string;
  name: string;
  url: string;
  /** favicon url or local asset; null falls back to the first letter */
  icon: string | null;
  builtin: boolean;
}

const STORAGE_KEY = "runjam.apps";

/** Apps pinned in the sidebar by default — just RunJam. */
export const DEFAULT_APPS: AppItem[] = [
  { id: "runjam", name: "RunJam", url: "https://runjam.app", icon: "/runjam-logo.svg", builtin: true },
];

/** Common apps offered in the Apps settings page for one-click adding. */
export const PRESET_APPS: AppItem[] = [
  { id: "preset-google", name: "Google", url: "https://www.google.com", icon: "https://www.google.com/favicon.ico", builtin: false },
  { id: "preset-gmail", name: "Gmail", url: "https://mail.google.com", icon: "https://mail.google.com/favicon.ico", builtin: false },
  { id: "preset-x", name: "X Twitter", url: "https://x.com", icon: "https://x.com/favicon.ico", builtin: false },
  { id: "preset-telegram", name: "Telegram Web", url: "https://web.telegram.org", icon: "https://web.telegram.org/favicon.ico", builtin: false },
];

function loadApps(): AppItem[] {
  let apps: AppItem[] = [];
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) {
      const parsed = JSON.parse(saved);
      if (Array.isArray(parsed)) apps = parsed as AppItem[];
    }
  } catch {
    // ignore storage errors
  }
  // Built-in apps must always be present (e.g. RunJam) — re-add if missing.
  for (const def of DEFAULT_APPS) {
    if (!apps.some((a) => a.id === def.id)) {
      apps.unshift({ ...def });
    }
  }
  return apps;
}

function persist(apps: AppItem[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(apps));
  } catch {
    // ignore storage errors
  }
}

/** Derive a favicon url for an arbitrary website. */
export function faviconFor(url: string): string | null {
  try {
    const hostname = new URL(url).hostname;
    if (!hostname) return null;
    return `https://www.google.com/s2/favicons?domain=${hostname}&sz=64`;
  } catch {
    return null;
  }
}

/**
 * Quick-launch web apps shown in the sidebar.
 * Persisted to localStorage; defaults to common web apps.
 */
export const useAppsStore = defineStore("apps", () => {
  const apps = ref<AppItem[]>(loadApps());

  watch(apps, (val) => persist(val), { deep: true });

  function addApp(name: string, url: string): AppItem {
    const item: AppItem = {
      id: `app-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      name: name.trim(),
      url: url.trim(),
      icon: faviconFor(url),
      builtin: false,
    };
    apps.value.push(item);
    return item;
  }

  function removeApp(id: string): void {
    const target = apps.value.find((a) => a.id === id);
    if (target?.builtin) return; // built-in apps are pinned and cannot be removed
    apps.value = apps.value.filter((a) => a.id !== id);
  }

  function resetDefaults(): void {
    apps.value = DEFAULT_APPS.map((a) => ({ ...a }));
  }

  return { apps, addApp, removeApp, resetDefaults };
});
