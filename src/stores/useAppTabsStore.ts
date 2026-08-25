import { ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AppItem } from "./useAppsStore";

export interface AppTab {
  id: string;
  name: string;
  url: string;
  icon: string | null;
  /** Label of the child webview backing this tab (created by `open_app_tab`). */
  webviewLabel: string;
}

/**
 * Browser-like app tabs inside the main window. Each open app gets a child
 * webview (created via the Rust `open_app_tab` command) laid out below the
 * top bar + tab strip; switching tabs shows/hides those webviews. "Home"
 * (activeTabId === null) shows the normal RunJam workspace.
 */
export const useAppTabsStore = defineStore("appTabs", () => {
  // Version marker so we can tell from the devtools console whether the
  // frontend is running this code at all.
  console.log("[app-tab] store v3 loaded");
  const tabs = ref<AppTab[]>([]);
  const activeTabId = ref<string | null>(null);
  let unlistenResize: (() => void) | null = null;

  function relayout() {
    // Keep every child webview aligned below the tab strip (window may have
    // been resized, tabs added/removed, etc.).
    invoke("layout_app_tabs").catch(() => {});
  }

  /** Dedupe rapid double-clicks while a tab is still being created. */
  const pendingOpens: Record<string, Promise<void> | undefined> = {};

  /** Open an app as a tab (or switch to it if already open). */
  function openApp(item: AppItem): Promise<void> {
    const inflight = pendingOpens[item.id];
    if (inflight) return inflight;
    const p = (async () => {
      const existing = tabs.value.find((t) => t.id === item.id);
      if (existing) {
        await activateTab(existing.id);
        return;
      }
      const webviewLabel = await invoke<string>("open_app_tab", {
        url: item.url,
      });
      tabs.value.push({
        id: item.id,
        name: item.name,
        url: item.url,
        icon: item.icon,
        webviewLabel,
      });
      await activateTab(item.id);
    })();
    pendingOpens[item.id] = p;
    p.finally(() => {
      delete pendingOpens[item.id];
    }).catch(() => {});
    return p;
  }

  /** Switch to an app tab: show its webview, hide the others. */
  async function activateTab(id: string) {
    activeTabId.value = id;
    await syncVisibility();
    relayout();
  }

  /** Switch back to the RunJam workspace (hide every app webview). */
  async function goHome() {
    activeTabId.value = null;
    await syncVisibility();
    relayout();
  }

  /** Hide every app webview WITHOUT changing the active tab. Used when
   *  leaving the workspace route so an app tab never covers other pages. */
  async function hideAll() {
    for (const t of tabs.value) {
      await invoke("set_app_tab_visible", { label: t.webviewLabel, visible: false }).catch(
        () => {}
      );
    }
  }

  /** Restore the previously active tab after returning to the workspace. */
  async function restore() {
    await syncVisibility();
    relayout();
  }

  /** Close an app tab (destroys its child webview) and activate a neighbor. */
  async function closeTab(id: string) {
    const idx = tabs.value.findIndex((t) => t.id === id);
    if (idx === -1) return;
    const tab = tabs.value[idx];
    await invoke("close_app_tab", { label: tab.webviewLabel }).catch(() => {});
    tabs.value.splice(idx, 1);
    if (activeTabId.value === id) {
      const next = tabs.value[Math.min(idx, tabs.value.length - 1)];
      if (next) {
        await activateTab(next.id);
      } else {
        activeTabId.value = null;
        await syncVisibility();
      }
    }
  }

  /**
   * Monotonic sequence guarding against interleaved tab switches: each call
   * captures its own sequence and bails out as soon as a newer switch has
   * happened, so a slow `activateTab` can never clobber a later `goHome`.
   */
  let visibilitySeq = 0;

  async function syncVisibility() {
    const seq = ++visibilitySeq;
    for (const t of tabs.value) {
      if (seq !== visibilitySeq) return;
      const visible = t.id === activeTabId.value;
      console.log(`[app-tab] syncVisibility: ${t.webviewLabel} -> ${visible}`);
      await invoke("set_app_tab_visible", { label: t.webviewLabel, visible })
        .then(() => console.log(`[app-tab] set_visible OK: ${t.webviewLabel}`))
        .catch((e) => console.error(`[app-tab] set_visible FAILED: ${t.webviewLabel}`, e));
    }
  }

  /** Re-align webviews whenever the window is resized. */
  function init() {
    if (unlistenResize) return;
    getCurrentWindow()
      .onResized(() => relayout())
      .then((fn) => (unlistenResize = fn))
      .catch(() => {});
  }

  function dispose() {
    unlistenResize?.();
    unlistenResize = null;
  }

  return {
    tabs,
    activeTabId,
    openApp,
    activateTab,
    goHome,
    hideAll,
    restore,
    closeTab,
    init,
    dispose,
    relayout,
  };
});
