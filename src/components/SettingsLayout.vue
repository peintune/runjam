<script setup lang="ts">
import { useRouter, useRoute } from "vue-router";
import { ArrowLeft, AppWindow, BarChart3, Bot, Cpu, Globe, Info, Settings, Sparkles } from "lucide-vue-next";
import type { TranslationKey } from "../i18n";
import WindowControls from "./WindowControls.vue";

const router = useRouter();
const route = useRoute();

const navItems: { path: string; labelKey: TranslationKey; icon: typeof Cpu }[] = [
  { path: "/settings/models/commercial", labelKey: "settings.commercialModels", icon: Globe },
  { path: "/settings/models", labelKey: "settings.localModels", icon: Cpu },
  { path: "/settings/apps", labelKey: "settings.apps", icon: AppWindow },
  { path: "/settings/agents", labelKey: "settings.agents", icon: Bot },
  { path: "/settings/costs", labelKey: "settings.costs", icon: BarChart3 },
  { path: "/settings/skills", labelKey: "settings.skills", icon: Sparkles },
  { path: "/settings/general", labelKey: "settings.general", icon: Settings },
  { path: "/settings/about", labelKey: "settings.about", icon: Info },
];
</script>

<template>
  <div class="flex flex-col h-screen bg-[#f8f9fb] dark:bg-[#101015]">
    <!-- drag region -->
    <div data-tauri-drag-region class="flex-shrink-0 h-8 w-full relative" style="-webkit-app-region: drag">
      <div class="absolute right-0 top-0">
        <WindowControls />
      </div>
    </div>

    <!-- body -->
    <div class="flex flex-1 min-h-0">
      <aside class="w-[220px] flex-shrink-0 flex flex-col mx-3 mt-0 mb-3 rounded-2xl bg-white border border-gray-200 shadow-sm overflow-hidden">
        <!-- Back button -->
        <div class="px-4 pt-3 pb-1">
          <button
            @click="router.push('/')"
            class="flex items-center gap-2 px-2 py-1.5 rounded-lg text-sm text-gray-500 hover:text-gray-700 hover:bg-gray-100 transition-colors w-full cursor-pointer"
          >
            <ArrowLeft :size="16" /> {{ $t("common.back") }}
          </button>
        </div>
        <div class="border-t border-gray-100 my-3 mx-4" />
        <div class="px-5 pb-3">
          <h2 class="text-[15px] font-semibold text-gray-900 tracking-tight">{{ $t("settings.title") }}</h2>
        </div>
        <nav class="flex-1 px-3 space-y-0.5">
          <button v-for="item in navItems" :key="item.path"
            @click="router.push(item.path)"
            :class="['w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-[13px] transition-colors duration-150 cursor-pointer',
              route.path === item.path ? 'bg-gray-100 text-gray-900 font-medium dark:bg-gray-200' : 'text-gray-500 hover:bg-gray-50 hover:text-gray-700']"
          >
            <component :is="item.icon" :size="18" />
            {{ $t(item.labelKey) }}
          </button>
        </nav>
      </aside>

      <div class="flex-1 overflow-auto">
        <router-view />
      </div>
    </div>
  </div>
</template>
