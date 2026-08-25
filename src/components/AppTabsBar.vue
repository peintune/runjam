<script setup lang="ts">
import { nextTick, ref } from "vue";
import { useRouter } from "vue-router";
import { Plus, X, ArrowLeft, ArrowRight, RotateCw } from "lucide-vue-next";
import { useAppTabsStore } from "../stores/useAppTabsStore";
import { faviconFor, type AppItem } from "../stores/useAppsStore";

const appTabs = useAppTabsStore();
const router = useRouter();

/** Switch back to the RunJam workspace: deactivate the app tab and make sure
 *  we are on the home route (also fine when opened from /board or settings). */
function goHome() {
  appTabs.goHome();
  if (router.currentRoute.value.path !== "/") {
    router.push("/");
  }
}

/** Inline URL input (browser-like): the "+" expands into an address bar. */
const showUrlInput = ref(false);
const urlInput = ref("");
const urlInputEl = ref<HTMLInputElement | null>(null);

function toggleUrlInput() {
  showUrlInput.value = !showUrlInput.value;
  if (showUrlInput.value) {
    urlInput.value = "";
    nextTick(() => urlInputEl.value?.focus());
  }
}

/** Open the typed address as a brand-new app tab. */
function submitUrl() {
  const raw = urlInput.value.trim();
  showUrlInput.value = false;
  if (!raw) return;
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `https://${raw}`;
  let parsed: URL;
  try {
    parsed = new URL(withScheme);
  } catch {
    return;
  }
  const item: AppItem = {
    id: "",
    name: parsed.hostname.replace(/^www\./, ""),
    url: parsed.toString(),
    icon: faviconFor(parsed.toString()),
    builtin: false,
  };
  appTabs.openApp(item).catch(console.error);
}
</script>

<template>
  <div
    v-if="appTabs.tabs.length > 0"
    class="flex items-center gap-1 h-9 flex-shrink-0 px-2 border-b border-gray-200/70 bg-white/95 backdrop-blur z-30 select-none"
    style="-webkit-app-region: no-drag"
  >
    <!-- Home tab: back to the RunJam workspace -->
    <button
      @click="goHome"
      class="flex items-center gap-1.5 h-7 px-2.5 rounded-lg text-[12px] font-medium transition-colors cursor-pointer"
      :class="
        appTabs.activeTabId === null
          ? 'bg-gray-200/70 text-gray-900'
          : 'text-gray-500 hover:bg-gray-100 hover:text-gray-700'
      "
      :title="$t('appsTab.home')"
    >
      <img src="/runjam-logo.svg" class="w-4 h-4 rounded" alt="RunJam" />
      <span class="hidden sm:inline">RunJam</span>
    </button>

    <div class="w-px h-4 bg-gray-200/80 mx-0.5 flex-shrink-0" />

    <!-- App tabs (browser style) -->
    <div
      class="flex items-center gap-1 min-w-0 flex-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
    >
      <button
        v-for="tab in appTabs.tabs"
        :key="tab.id"
        @click="appTabs.activateTab(tab.id)"
        @click.middle.prevent="appTabs.closeTab(tab.id)"
        class="group flex items-center gap-1.5 h-7 pl-2 pr-1 rounded-lg text-[12px] whitespace-nowrap shrink-0 transition-colors cursor-pointer"
        :class="
          appTabs.activeTabId === tab.id
            ? 'bg-gray-200/70 text-gray-900'
            : 'text-gray-500 hover:bg-gray-100 hover:text-gray-700'
        "
        :title="tab.name"
      >
        <img
          v-if="tab.icon"
          :src="tab.icon"
          class="w-4 h-4 rounded"
          alt=""
          @error="tab.icon = null"
        />
        <span
          v-else
          class="w-4 h-4 rounded flex items-center justify-center text-[10px] font-semibold bg-gray-200 text-gray-600"
          >{{ tab.name.charAt(0).toUpperCase() }}</span
        >
        <span class="max-w-[140px] truncate">{{ tab.name }}</span>
        <span
          @click.stop="appTabs.closeTab(tab.id)"
          class="w-4 h-4 flex items-center justify-center rounded text-gray-400 opacity-0 group-hover:opacity-100 hover:bg-gray-300/60 hover:text-gray-700 transition-all cursor-pointer"
          :title="$t('appsTab.close')"
        >
          <X :size="11" />
        </span>
      </button>
    </div>

    <!-- Add tab: the "+" expands into a browser-like URL input -->
    <input
      v-if="showUrlInput"
      ref="urlInputEl"
      v-model="urlInput"
      type="text"
      @keyup.enter="submitUrl"
      @keyup.esc="showUrlInput = false"
      @blur="showUrlInput = false"
      :placeholder="$t('appsTab.urlPlaceholder')"
      class="w-44 h-7 flex-shrink-0 px-2 rounded-lg border border-gray-200 bg-white text-gray-900 placeholder:text-gray-400 outline-none focus:border-blue-400 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100 dark:placeholder:text-zinc-500 dark:focus:border-blue-500 transition-colors"
    />
    <button
      v-else
      @click="toggleUrlInput"
      class="w-7 h-7 flex-shrink-0 flex items-center justify-center rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer"
      :title="$t('sidebar.addApp')"
    >
      <Plus :size="14" />
    </button>

    <!-- Browser navigation for the active app tab -->
    <div class="w-px h-4 bg-gray-200/80 mx-0.5 flex-shrink-0" />
    <button
      @click="appTabs.navigate('back')"
      :disabled="appTabs.activeTabId === null"
      class="w-7 h-7 flex-shrink-0 flex items-center justify-center rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer disabled:cursor-default disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-gray-400"
      :title="$t('appsTab.back')"
    >
      <ArrowLeft :size="14" />
    </button>
    <button
      @click="appTabs.navigate('forward')"
      :disabled="appTabs.activeTabId === null"
      class="w-7 h-7 flex-shrink-0 flex items-center justify-center rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer disabled:cursor-default disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-gray-400"
      :title="$t('appsTab.forward')"
    >
      <ArrowRight :size="14" />
    </button>
    <button
      @click="appTabs.navigate('reload')"
      :disabled="appTabs.activeTabId === null"
      class="w-7 h-7 flex-shrink-0 flex items-center justify-center rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer disabled:cursor-default disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-gray-400"
      :title="$t('appsTab.reload')"
    >
      <RotateCw :size="13" />
    </button>
  </div>
</template>
