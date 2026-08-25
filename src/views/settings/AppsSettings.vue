<script setup lang="ts">
import { ref, computed } from "vue";
import { useRouter } from "vue-router";
import { Plus, ExternalLink, Trash2, RotateCcw } from "lucide-vue-next";
import { useAppsStore, DEFAULT_APPS, PRESET_APPS, type AppItem } from "@/stores/useAppsStore";
import { useAppTabsStore } from "@/stores/useAppTabsStore";

const store = useAppsStore();
const appTabs = useAppTabsStore();
const router = useRouter();

const name = ref("");
const url = ref("");
const error = ref("");

/** Preset apps that aren't in the current list yet. */
const availablePresets = computed(() => {
  const existing = new Set(store.apps.map((a) => a.url));
  return PRESET_APPS.filter((p) => !existing.has(p.url));
});

function isValidUrl(raw: string): boolean {
  try {
    const u = new URL(raw.trim());
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

function handleAdd() {
  error.value = "";
  if (!name.value.trim()) {
    error.value = "name";
    return;
  }
  if (!isValidUrl(url.value)) {
    error.value = "url";
    return;
  }
  store.addApp(name.value, url.value);
  name.value = "";
  url.value = "";
}

function addPreset(preset: (typeof PRESET_APPS)[number]) {
  store.addApp(preset.name, preset.url);
}

async function handleOpen(app: AppItem) {
  try {
    await appTabs.openApp(app);
    // The tab lives in the workspace view — jump back so it's visible.
    router.push("/");
  } catch (e) {
    console.error("Failed to open app:", e);
  }
}

function handleRemove(id: string) {
  store.removeApp(id);
}
</script>

<template>
  <div class="p-6 flex justify-center">
    <div class="max-w-2xl w-full">
      <div class="flex items-center justify-between">
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight mb-1">{{ $t("settings.apps") }}</h2>
        <button
          v-if="store.apps.length !== DEFAULT_APPS.length"
          class="inline-flex items-center gap-1 px-2.5 py-1 text-[12px] font-medium text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded-lg transition-colors cursor-pointer"
          @click="store.resetDefaults()"
        >
          <RotateCcw :size="13" />
          {{ $t("apps.restoreDefaults") }}
        </button>
      </div>
      <p class="text-[12px] text-gray-400 mb-6">{{ $t("apps.desc") }}</p>

      <!-- preset apps -->
      <div v-if="availablePresets.length > 0" class="bg-white rounded-xl border border-gray-100 p-5 mb-6">
        <p class="text-[14px] font-medium text-gray-900 mb-3">{{ $t("apps.popular") }}</p>
        <div class="flex flex-wrap gap-2">
          <button
            v-for="preset in availablePresets"
            :key="preset.id"
            class="inline-flex items-center gap-2 pl-1.5 pr-3 py-1.5 rounded-lg border border-gray-200 hover:border-gray-300 hover:bg-gray-50 transition-colors cursor-pointer"
            @click="addPreset(preset)"
          >
            <img v-if="preset.icon" :src="preset.icon" :alt="preset.name" class="w-5 h-5 rounded" @error="preset.icon = null" />
            <span v-else class="w-5 h-5 rounded flex items-center justify-center text-[11px] font-semibold text-gray-500 bg-gray-100">{{ preset.name.charAt(0) }}</span>
            <span class="text-[13px] font-medium text-gray-700">{{ preset.name }}</span>
            <Plus :size="13" class="text-gray-400" />
          </button>
        </div>
      </div>

      <!-- add custom app -->
      <div class="bg-white rounded-xl border border-gray-100 p-5 mb-6">
        <p class="text-[14px] font-medium text-gray-900 mb-3">{{ $t("apps.addCustom") }}</p>
        <div class="flex flex-col gap-2">
          <input
            v-model="name"
            type="text"
            spellcheck="false"
            :placeholder="$t('apps.namePlaceholder')"
            class="h-9 px-3 rounded-lg border text-[13px] text-gray-900 placeholder:text-gray-300 bg-white focus:outline-none transition-colors"
            :class="error === 'name' ? 'border-red-300 focus:border-red-400' : 'border-gray-200 focus:border-blue-400'"
            @keyup.enter="handleAdd"
          />
          <p v-if="error === 'name'" class="text-[12px] text-red-600">{{ $t("apps.nameRequired") }}</p>
          <input
            v-model="url"
            type="text"
            spellcheck="false"
            :placeholder="$t('apps.urlPlaceholder')"
            class="h-9 px-3 rounded-lg border text-[13px] text-gray-900 placeholder:text-gray-300 bg-white focus:outline-none transition-colors"
            :class="error === 'url' ? 'border-red-300 focus:border-red-400' : 'border-gray-200 focus:border-blue-400'"
            @keyup.enter="handleAdd"
          />
          <p v-if="error === 'url'" class="text-[12px] text-red-600">{{ $t("apps.invalidUrl") }}</p>
          <button
            class="mt-1 self-start inline-flex items-center gap-1.5 px-4 py-2 rounded-lg text-[12px] font-semibold text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm"
            @click="handleAdd"
          >
            <Plus :size="14" />
            {{ $t("apps.add") }}
          </button>
        </div>
      </div>

      <!-- app list -->
      <div v-if="store.apps.length > 0" class="bg-white rounded-xl border border-gray-100 divide-y divide-gray-100">
        <div v-for="app in store.apps" :key="app.id" class="flex items-center gap-3 px-5 py-3">
          <div class="w-9 h-9 rounded-xl bg-gray-50 border border-gray-100 flex items-center justify-center flex-shrink-0">
            <img v-if="app.icon" :src="app.icon" :alt="app.name" class="w-5 h-5 rounded" @error="app.icon = null" />
            <span v-else class="text-[12px] font-semibold text-gray-500">{{ app.name.charAt(0).toUpperCase() }}</span>
          </div>
          <div class="flex-1 min-w-0">
            <p class="text-[13px] font-medium text-gray-900 truncate">{{ app.name }}</p>
            <p class="text-[12px] text-gray-400 truncate">{{ app.url }}</p>
          </div>
          <span v-if="app.builtin" class="px-2 py-0.5 text-[11px] font-medium text-gray-400 bg-gray-100 rounded-full flex-shrink-0">
            {{ $t("apps.builtin") }}
          </span>
          <button
            class="p-2 rounded-lg text-gray-400 hover:text-blue-600 hover:bg-blue-50 transition-colors cursor-pointer flex-shrink-0"
            :title="$t('apps.open')"
            @click="handleOpen(app)"
          >
            <ExternalLink :size="15" />
          </button>
          <button
            v-if="!app.builtin"
            class="p-2 rounded-lg text-gray-400 hover:text-red-500 hover:bg-red-50 transition-colors cursor-pointer flex-shrink-0"
            :title="$t('apps.delete')"
            @click="handleRemove(app.id)"
          >
            <Trash2 :size="15" />
          </button>
        </div>
      </div>
      <p v-else class="text-center text-[13px] text-gray-400 py-10">{{ $t("apps.empty") }}</p>
    </div>
  </div>
</template>
